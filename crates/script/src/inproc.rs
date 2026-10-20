//! In-process scripts as studies: a language's VM ([`Vm`]) runs the
//! script; the [`host`](crate::host) owns its meaning and state. Each study
//! gets its own VM (so optimizer runs are independent and parallel); the
//! forming bar's copy shares its study's VM and restores the `state`
//! snapshot it needs before running.

use crate::host::{Decl, HostState, LOADTIME, ScriptError, SymInfo, Val, color_fn, dry_bars, next_ver};
use opentern_core::Timeframe;
use opentern_engine::Props;
use opentern_series::study::{Ctx, Factory, Inputs, Meta, Out, Study};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// What the language's host functions see: declarations while the top
/// level runs, the bar's state while `on_bar` runs.
pub enum Phase {
    Idle,
    Load { decl: Decl, inputs: Inputs },
    Run(Box<HostState>),
}

pub type Shared = Arc<Mutex<Phase>>;

/// Calls a host function by name, whichever phase the script is in.
pub fn dispatch(shared: &Shared, name: &str, args: &[Val]) -> Result<Val, String> {
    let mut p = shared.lock().unwrap_or_else(|e| e.into_inner());
    match &mut *p {
        Phase::Load { decl, inputs } => match name.strip_prefix("color.") {
            Some(c) => color_fn(c, args),
            None => decl.call(name, args, inputs),
        },
        Phase::Run(h) => {
            if LOADTIME.contains(&name) {
                return Err(format!("{name}() goes at the top of the script, not inside on_bar."));
            }
            h.call(name, args)
        }
        Phase::Idle => Err("The script called a function after it finished.".into()),
    }
}

/// Runs `f` on the bar's state; an error outside `on_bar`.
pub fn with_run<R>(
    shared: &Shared,
    name: &str,
    f: impl FnOnce(&mut HostState) -> Result<R, String>,
) -> Result<R, String> {
    let mut p = shared.lock().unwrap_or_else(|e| e.into_inner());
    match &mut *p {
        Phase::Run(h) => f(h),
        _ => Err(format!("{name}() goes inside on_bar, not at the top of the script.")),
    }
}

/// `ta.*` results with more than one value.
pub fn ta_multi(name: &str) -> bool {
    matches!(name, "macd" | "bb" | "dmi" | "supertrend")
}

/// A script loaded in one language's VM.
pub trait Vm: Send {
    /// Runs `on_bar` with the bar's fields (see `host::BAR_NUMS`). VMs
    /// reuse one `bar` table/object, so this allocates nothing per bar.
    fn on_bar(&mut self, nums: &[f64; 11], flags: &[bool; 3]) -> Result<(), ScriptError>;
    /// The `state` table.
    fn get_state(&mut self) -> Result<Val, ScriptError>;
    fn set_state(&mut self, v: &Val) -> Result<(), ScriptError>;
}

/// A language: makes a VM and runs the script's top level in it.
pub trait Lang: Send + Sync + 'static {
    /// "luau", "js": the study id prefix.
    fn key(&self) -> &'static str;
    fn create(&self, src: &str, file: &str, shared: Shared, syminfo: &SymInfo) -> Result<Box<dyn Vm>, ScriptError>;
}

/// Runs `on_bar` once for the bar `host` has begun.
fn run_bar(
    vm: &Mutex<Box<dyn Vm>>,
    vm_ver: &AtomicU64,
    shared: &Shared,
    host: &mut Option<Box<HostState>>,
) -> Result<(), ScriptError> {
    let mut vm = vm.lock().unwrap_or_else(|e| e.into_inner());
    let h = host.as_mut().expect("host state");
    if vm_ver.load(Ordering::Relaxed) != h.state_ver {
        vm.set_state(&h.state)?;
        vm_ver.store(h.state_ver, Ordering::Relaxed);
    }
    let (nums, flags) = h.bar_fields();
    *shared.lock().unwrap_or_else(|e| e.into_inner()) = Phase::Run(host.take().expect("host state"));
    let r = vm.on_bar(&nums, &flags);
    let back = std::mem::replace(&mut *shared.lock().unwrap_or_else(|e| e.into_inner()), Phase::Idle);
    let Phase::Run(h) = back else {
        unreachable!("the phase only changes here")
    };
    *host = Some(h);
    r?;
    let h = host.as_mut().expect("host state");
    h.state = vm.get_state()?;
    h.state_ver = next_ver();
    vm_ver.store(h.state_ver, Ordering::Relaxed);
    Ok(())
}

/// A compiled script: declarations known, plots found by a dry run.
#[derive(Clone)]
pub struct Compiled {
    lang: Arc<dyn Lang>,
    src: Arc<str>,
    file: Arc<str>,
    pub decl: Arc<Decl>,
    meta: &'static Meta,
    syminfo: SymInfo,
    props: Option<serde_json::Value>,
}

/// Loads a script, reads its declarations and finds its plots.
pub fn compile(lang: Arc<dyn Lang>, src: &str, file: &str, syminfo: SymInfo) -> Result<Compiled, ScriptError> {
    let shared: Shared = Arc::new(Mutex::new(Phase::Load {
        decl: Decl::default(),
        inputs: Inputs::default(),
    }));
    let vm = lang.create(src, file, shared.clone(), &syminfo)?;
    let Phase::Load { decl, .. } =
        std::mem::replace(&mut *shared.lock().unwrap_or_else(|e| e.into_inner()), Phase::Idle)
    else {
        unreachable!()
    };
    if !decl.declared {
        return Err(ScriptError::new(
            1,
            "Start the script with indicator(\"Name\") or strategy(\"Name\").",
        ));
    }
    let props = decl.strategy.then(|| decl.props.clone());
    // The dry run: plots are found in the order they're called.
    let mut host = Some(Box::new(HostState::new(
        &decl,
        props
            .as_ref()
            .map(|p| Props::from_json(p, syminfo.mintick, syminfo.mincontract())),
        syminfo.clone(),
    )));
    if let Some(h) = &mut host {
        h.discovering = true;
    }
    let vm = Mutex::new(vm);
    let ver = AtomicU64::new(0);
    let bars = dry_bars(60);
    let tf = Timeframe::parse("1h").expect("valid");
    for (i, b) in bars.iter().enumerate() {
        if let Some(h) = &mut host {
            h.begin(b, i, false, bars.len(), i == 0, tf);
        }
        run_bar(&vm, &ver, &shared, &mut host)?;
        if let Some(h) = &mut host {
            h.end(b);
        }
    }
    let mut decl = decl;
    decl.plots = host.map(|h| h.plots).unwrap_or_default();
    let meta = decl.meta(&format!("{}:{file}", lang.key()));
    Ok(Compiled {
        lang,
        src: src.into(),
        file: file.into(),
        decl: Arc::new(decl),
        meta,
        syminfo,
        props,
    })
}

impl std::fmt::Debug for Compiled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Compiled({}:{})", self.lang.key(), self.file)
    }
}

impl Compiled {
    pub fn meta(&self) -> &'static Meta {
        self.meta
    }

    pub fn is_strategy(&self) -> bool {
        self.decl.strategy
    }

    /// `strategy(...)`'s options, as JSON.
    pub fn strategy_props(&self) -> Option<&serde_json::Value> {
        self.props.as_ref()
    }

    /// The same script with some strategy properties replaced.
    pub fn with_props(&self, over: &serde_json::Value) -> Compiled {
        let mut c = self.clone();
        if let Some(p) = &mut c.props {
            *p = opentern_engine::merge_json(p, over);
        }
        c
    }

    /// A study for given inputs (each with its own VM).
    pub fn study(&self, inputs: &Inputs) -> ScriptStudy {
        ScriptStudy::new(self, inputs)
    }

    pub fn factory(&self) -> Factory {
        let c = self.clone();
        Arc::new(move |inputs: &Inputs| Box::new(c.study(inputs)) as Box<dyn Study>)
    }
}

/// A script running on a series.
#[derive(Clone)]
pub struct ScriptStudy {
    vm: Option<Arc<Mutex<Box<dyn Vm>>>>,
    vm_ver: Arc<AtomicU64>,
    shared: Shared,
    host: Option<Box<HostState>>,
    error: Option<ScriptError>,
}

impl ScriptStudy {
    fn new(c: &Compiled, inputs: &Inputs) -> ScriptStudy {
        let shared: Shared = Arc::new(Mutex::new(Phase::Load {
            decl: Decl::default(),
            inputs: inputs.clone(),
        }));
        let props = c
            .props
            .as_ref()
            .map(|p| Props::from_json(p, c.syminfo.mintick, c.syminfo.mincontract()));
        let host = Some(Box::new(HostState::new(&c.decl, props, c.syminfo.clone())));
        let mut s = ScriptStudy {
            vm: None,
            vm_ver: Arc::new(AtomicU64::new(0)),
            shared: shared.clone(),
            host,
            error: None,
        };
        match c.lang.create(&c.src, &c.file, shared.clone(), &c.syminfo) {
            Ok(vm) => s.vm = Some(Arc::new(Mutex::new(vm))),
            Err(e) => s.error = Some(e),
        }
        *shared.lock().unwrap_or_else(|e| e.into_inner()) = Phase::Idle;
        s
    }

    pub fn error(&self) -> Option<&ScriptError> {
        self.error.as_ref()
    }

    pub fn broker(&self) -> Option<&opentern_engine::Broker> {
        self.host.as_ref().and_then(|h| h.broker.as_ref())
    }

    pub fn logs(&self) -> &[String] {
        self.host.as_ref().map(|h| h.logs.as_slice()).unwrap_or(&[])
    }

    pub fn alerts(&self) -> &[crate::host::Alert] {
        self.host.as_ref().map(|h| h.alerts.as_slice()).unwrap_or(&[])
    }
}

impl Study for ScriptStudy {
    fn step(&mut self, c: &Ctx, o: &mut Out) {
        if self.error.is_some() {
            return;
        }
        let Some(vm) = self.vm.clone() else { return };
        if let Some(h) = &mut self.host {
            h.begin(c.bar, c.index, c.forming, c.count, c.new_session, c.tf);
        }
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            run_bar(&vm, &self.vm_ver, &self.shared, &mut self.host)
        }));
        let r = r.unwrap_or_else(|_| Err(ScriptError::new(0, "The script crashed.")));
        if let Err(e) = r {
            self.error = Some(e);
            return;
        }
        let Some(h) = &mut self.host else { return };
        h.end(c.bar);
        for (d, v) in o.values.iter_mut().zip(&h.values) {
            *d = *v;
        }
        for (d, v) in o.raw.iter_mut().zip(&h.raw) {
            *d = *v;
        }
        o.bg = h.bg;
        o.bar = h.barcolor;
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }
}

/// The 1-based line of the first `name:NN` in an error text.
pub fn line_in(text: &str, name: &str) -> usize {
    let mut rest = text;
    while let Some(i) = rest.find(name) {
        let after = &rest[i + name.len()..];
        if let Some(digits) = after.strip_prefix(':') {
            let n: String = digits.chars().take_while(|c| c.is_ascii_digit()).collect();
            if let Ok(v) = n.parse() {
                return v;
            }
        }
        rest = &rest[i + name.len()..];
    }
    0
}

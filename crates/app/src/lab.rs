//! The Strategy Lab tab: the quant tools (`opentern_engine::quant`) on the
//! chart's strategy. Optimize (grid, random or Bayesian, as a list or a
//! heatmap), Walk-forward, Monte Carlo and Portfolio (one strategy on a
//! watchlist or preset). Long runs go to a background thread (which fans
//! out on rayon) with a progress bar and Stop; the UI never waits.
//!
//! Strategies are rerun headlessly from the chart study's own factory, so
//! any in-process language (Pine, Luau, JS) works. Python strategies run in
//! their own process and aren't rerun here.

use crate::app::App;
use crate::icons::ph;
use crate::shell::small_button;
use crate::strategy::{Fmt, closed_broker, kpi, num, pct, table, time_text, tone};
use crate::theme::{self, Theme};
use egui::{Align, Align2, Color32, CornerRadius, Layout, Rect, Sense, Stroke, Ui, pos2, vec2};
use opentern_core::{Bar, Symbol, Timeframe};
use opentern_engine::Broker;
use opentern_engine::quant::montecarlo::{McResult, McSetup, Resample};
use opentern_engine::quant::optimize::{self, Method, Outcome, Setup};
use opentern_engine::quant::portfolio::{self, PortfolioResult};
use opentern_engine::quant::walkforward::{self, WalkResult, WalkSetup};
use opentern_engine::quant::{Objective, PValue, Param, Progress, Range, export};
use opentern_series::study::{Factory, InputKind, Inputs, Instance, Meta, Source, Value, sessions};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{Receiver, channel};
use std::time::Instant;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Mode {
    #[default]
    Optimize,
    WalkForward,
    MonteCarlo,
    Portfolio,
}

impl Mode {
    const ALL: [Mode; 4] = [Mode::Optimize, Mode::WalkForward, Mode::MonteCarlo, Mode::Portfolio];

    fn title(self) -> &'static str {
        match self {
            Mode::Optimize => "Optimize",
            Mode::WalkForward => "Walk-forward",
            Mode::MonteCarlo => "Monte Carlo",
            Mode::Portfolio => "Portfolio",
        }
    }

    fn file(self) -> &'static str {
        match self {
            Mode::Optimize => "optimizer",
            Mode::WalkForward => "walk-forward",
            Mode::MonteCarlo => "monte carlo",
            Mode::Portfolio => "portfolio",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum MethodKind {
    #[default]
    Grid,
    Random,
    Bayesian,
}

/// One input in the parameter table.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ParamRow {
    pub id: String,
    pub title: String,
    pub on: bool,
    pub range: Range,
}

/// Saved between launches (settings key `lab`).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct LabSetup {
    pub mode: Mode,
    pub method: MethodKind,
    pub trials: usize,
    pub objective: Objective,
    pub min_trades: usize,
    pub in_sample: usize,
    pub out_sample: usize,
    pub anchored: bool,
    pub mc: McSetup,
    pub universe: crate::screener::Universe,
    pub port_bars: usize,
    /// Parameter tables by study id (`pine:file.pine`, …).
    pub params: HashMap<String, Vec<ParamRow>>,
}

impl Default for LabSetup {
    fn default() -> Self {
        LabSetup {
            mode: Mode::Optimize,
            method: MethodKind::Grid,
            trials: 150,
            objective: Objective::NetProfit,
            min_trades: 1,
            in_sample: 500,
            out_sample: 100,
            anchored: false,
            mc: McSetup::default(),
            universe: crate::screener::Universe::Preset("Crypto majors".into()),
            port_bars: 2000,
            params: HashMap::new(),
        }
    }
}

enum Done {
    Opt(Result<(Setup, Outcome), String>),
    Walk(Result<(Setup, WalkResult), String>),
    Mc(Result<McResult, String>),
    Port(Result<(PortfolioResult, Vec<String>), String>),
}

struct Job {
    progress: Arc<Progress>,
    rx: Receiver<Done>,
    started: Instant,
}

/// Bars being fetched for a portfolio run.
struct PortFetch {
    seq_base: u64,
    tf: Timeframe,
    want: Vec<Symbol>,
    got: Vec<(Symbol, Result<Vec<Bar>, String>)>,
    target: Target,
    capital: f64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum OptView {
    #[default]
    List,
    Heatmap,
}

/// The tab's state.
#[derive(Default)]
pub struct Lab {
    pub setup: LabSetup,
    loaded: bool,
    job: Option<Job>,
    fetch: Option<PortFetch>,
    runs: u64,
    pub status: String,
    opt: Option<(Setup, Outcome, u64)>,
    walk: Option<(Setup, WalkResult)>,
    mc: Option<McResult>,
    port: Option<PortfolioResult>,
    port_errors: Vec<String>,
    view: OptView,
    heat: (usize, usize),
    selected: Option<usize>,
    /// The study the running optimization is for.
    pending_uid: u64,
    /// For stills: run once the strategy is on the chart.
    autorun: bool,
    /// Height of the results area (charts size to it).
    view_h: f32,
}

/// A symbol's bars and its strategy run, or why it couldn't run.
type LegRun = Result<(Vec<Bar>, Broker), String>;

/// How to build the strategy: on the chart (its factory), or compiled
/// afresh for another symbol (with properties overridden).
pub(crate) type Build =
    Arc<dyn Fn(&Symbol, &serde_json::Value) -> Result<(&'static Meta, Factory), String> + Send + Sync>;

#[derive(Clone)]
struct Target {
    uid: u64,
    meta: &'static Meta,
    inputs: Inputs,
    factory: Factory,
    build: Option<Build>,
    tf: Timeframe,
    tz: jiff::tz::TimeZone,
    /// The chart's forming bar's time: a run whose bars end on it skips
    /// the strategy's signals there, like the Strategy Report.
    forming_at: Option<i64>,
}

fn to_value(spec: Option<&InputKind>, v: &PValue) -> Value {
    match (spec, v) {
        (Some(InputKind::Float { .. }), PValue::Int(x)) => Value::Float(*x as f64),
        (_, PValue::Int(x)) => Value::Int(*x),
        (_, PValue::Float(x)) => Value::Float(*x),
        (_, PValue::Bool(x)) => Value::Bool(*x),
        (_, PValue::Text(s)) => Value::Text(s.clone()),
    }
}

/// `base` with `values` set for `params`.
fn inputs_with(meta: &Meta, base: &Inputs, params: &[Param], values: &[PValue]) -> Inputs {
    let mut ins = base.clone();
    for (p, v) in params.iter().zip(values) {
        ins.set(&p.id, to_value(meta.input(&p.id).map(|s| &s.kind), v));
    }
    ins
}

/// Runs the study on `bars` and returns its broker.
fn run_study(
    meta: &'static Meta,
    inputs: &Inputs,
    factory: &Factory,
    bars: &[Bar],
    tf: Timeframe,
    tz: &jiff::tz::TimeZone,
    forming_at: Option<i64>,
) -> Option<Broker> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut inst = Instance::with_factory(meta, inputs, factory.clone());
        let sess = sessions(bars, tf, tz);
        inst.recompute(bars, &sess, tf);
        let forming = forming_at.is_some() && bars.last().map(|b| b.time) == forming_at;
        closed_broker(&inst, bars, forming)
    }))
    .ok()
    .flatten()
}

/// A sensible starting range for an input.
fn default_row(spec: &opentern_series::study::InputSpec) -> Option<ParamRow> {
    let range = match (&spec.kind, &spec.default) {
        (InputKind::Int { min, max }, d) => {
            let d = match d {
                Value::Int(v) => *v,
                Value::Float(v) => *v as i64,
                _ => 10,
            };
            let (from, to) = if d > 0 { ((d / 2).max(1), d * 2) } else { (0, 10) };
            let (from, to) = (from.clamp(*min, *max), to.clamp(*min, *max));
            Range::Int {
                from,
                to,
                step: ((to - from) / 10).max(1),
            }
        }
        (InputKind::Float { min, max, step }, d) => {
            let d = match d {
                Value::Float(v) => *v,
                Value::Int(v) => *v as f64,
                _ => 1.0,
            };
            let (from, to) = if d != 0.0 { (d / 2.0, d * 2.0) } else { (0.0, 1.0) };
            let (from, to) = (from.min(to).clamp(*min, *max), from.max(to).clamp(*min, *max));
            let raw = (to - from) / 10.0;
            let mag = 10f64.powf(raw.abs().max(1e-9).log10().floor());
            let st = ((raw / mag).round().max(1.0) * mag).max(*step);
            Range::Float { from, to, step: st }
        }
        (InputKind::Bool, _) => Range::Bool,
        (InputKind::Choice(opts), _) => Range::Choice(opts.iter().map(|s| s.to_string()).collect()),
        (InputKind::Source, _) => Range::Choice(Source::ALL.iter().map(|s| s.name().to_string()).collect()),
        _ => return None,
    };
    Some(ParamRow {
        id: spec.id.to_string(),
        title: spec.title.to_string(),
        on: false,
        range,
    })
}

fn safe_name(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_alphanumeric() || c == ' ' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

impl App {
    fn lab_load(&mut self) {
        if self.lab.loaded {
            return;
        }
        self.lab.loaded = true;
        if let Some(s) = self
            .store
            .value("lab")
            .and_then(|j| serde_json::from_str::<LabSetup>(&j).ok())
        {
            self.lab.setup = s;
        }
    }

    fn lab_save(&mut self) {
        if let Ok(j) = serde_json::to_string(&self.lab.setup) {
            let _ = self.store.set_value("lab", &j);
        }
    }

    /// The strategy the lab works on: the one picked in the report.
    fn lab_target(&self) -> Result<Target, String> {
        let Some(run) = self.strategies.current() else {
            return Err(
                "Add a strategy to the chart first. A strategy is a script that starts with strategy(…).".into(),
            );
        };
        let study = self
            .chart
            .studies
            .iter()
            .find(|s| s.uid == run.uid)
            .ok_or("The strategy isn't on the chart any more.")?;
        let Some(factory) = study.inst.factory() else {
            return Err(
                "Python strategies run in their own process, so the lab can't rerun them yet. Pine, Luau and JavaScript strategies work."
                    .into(),
            );
        };
        let script = self.scripts.iter().find(|r| r.uid == Some(run.uid));
        let build = script.and_then(|s| self.strategy_builder(s));
        Ok(Target {
            uid: run.uid,
            meta: study.inst.meta,
            inputs: study.inst.inputs.clone(),
            factory,
            build,
            tf: self.chart.tf,
            tz: self.chart.tz.clone(),
            forming_at: crate::strategy::last_bar_forming(
                &self.chart.symbol,
                self.chart.tf,
                &self.chart.bars,
                self.chart.replay.is_some(),
            )
            .then(|| self.chart.bars.last().map(|b| b.time))
            .flatten(),
        })
    }

    /// Compiles a script for another symbol with properties overridden
    /// (portfolio runs), whatever its language.
    fn strategy_builder(&self, run: &crate::scripting::ScriptRun) -> Option<Build> {
        let src = std::fs::read_to_string(self.scripts_dir().join(&run.file)).ok()?;
        let file = run.file.clone();
        let over = self.strategy_overrides(&file);
        match run.lang {
            crate::scripting::Lang::Pine => {
                let libraries = self.pine_env_for(&self.chart.symbol).libraries;
                let envs: Arc<dyn Fn(&Symbol) -> opentern_pine::Env + Send + Sync> = {
                    let libs = libraries.clone();
                    Arc::new(move |s: &Symbol| {
                        let mut e = crate::pine_host::env_of(s);
                        e.libraries = libs.clone();
                        e
                    })
                };
                Some(Arc::new(move |s: &Symbol, extra: &serde_json::Value| {
                    let script = opentern_pine::compile(&src, &file, envs(s)).map_err(|e| e.to_string())?;
                    let script = script.with_props(&opentern_engine::merge_json(&over, extra));
                    Ok((script.meta(), script.factory()))
                }))
            }
            _ => crate::scripting::other_builder(run.lang, src, file, over),
        }
    }

    /// Keeps the parameter table in step with the strategy's inputs.
    fn lab_params(&mut self, meta: &'static Meta) -> &mut Vec<ParamRow> {
        let fresh: Vec<ParamRow> = meta.inputs.iter().filter_map(default_row).collect();
        let rows = self.lab.setup.params.entry(meta.id.to_string()).or_default();
        let same = rows.len() == fresh.len() && rows.iter().zip(&fresh).all(|(a, b)| a.id == b.id);
        if !same {
            let old = std::mem::take(rows);
            *rows = fresh
                .into_iter()
                .map(|f| old.iter().find(|o| o.id == f.id).cloned().unwrap_or(f))
                .collect();
        }
        rows
    }

    fn lab_opt_setup(&mut self, meta: &'static Meta) -> Setup {
        let s = self.lab.setup.clone();
        let params = self
            .lab_params(meta)
            .iter()
            .filter(|r| r.on)
            .map(|r| Param {
                id: r.id.clone(),
                title: r.title.clone(),
                range: r.range.clone(),
            })
            .collect();
        Setup {
            params,
            method: match s.method {
                MethodKind::Grid => Method::Grid,
                MethodKind::Random => Method::Random { trials: s.trials },
                MethodKind::Bayesian => Method::Bayesian { trials: s.trials },
            },
            objective: s.objective,
            min_trades: s.min_trades,
            seed: 42,
        }
    }

    fn lab_start(&mut self, f: impl FnOnce(Arc<Progress>) -> Done + Send + 'static) {
        let progress = Arc::new(Progress::default());
        let (tx, rx) = channel();
        let p = progress.clone();
        let ctx = self.egui_ctx.clone();
        let spawned = std::thread::Builder::new().name("lab".into()).spawn(move || {
            let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(p)));
            let done = r.unwrap_or_else(|_| Done::Opt(Err("Something went wrong in the run.".into())));
            let _ = tx.send(done);
            ctx.request_repaint();
        });
        if spawned.is_err() {
            self.lab.status = "Couldn't start the run.".into();
            return;
        }
        self.lab.job = Some(Job {
            progress,
            rx,
            started: Instant::now(),
        });
        self.lab.status = "Running…".into();
    }

    pub(crate) fn lab_run(&mut self) {
        if self.lab.job.is_some() || self.lab.fetch.is_some() {
            return;
        }
        self.lab_save();
        let target = match self.lab_target() {
            Ok(t) => t,
            Err(e) => {
                self.lab.status = e;
                return;
            }
        };
        let bars = Arc::new(self.chart.bars.clone());
        match self.lab.setup.mode {
            Mode::Optimize => {
                let setup = self.lab_opt_setup(target.meta);
                self.lab.selected = None;
                self.lab.pending_uid = target.uid;
                self.lab_start(move |p| {
                    let run = runner(&target, &setup.params);
                    Done::Opt(optimize::optimize(&setup, &bars, &run, &p).map(|o| (setup, o)))
                });
            }
            Mode::WalkForward => {
                let w = WalkSetup {
                    optimize: self.lab_opt_setup(target.meta),
                    in_sample: self.lab.setup.in_sample,
                    out_sample: self.lab.setup.out_sample,
                    anchored: self.lab.setup.anchored,
                };
                self.lab_start(move |p| {
                    let run = runner(&target, &w.optimize.params);
                    Done::Walk(walkforward::walk_forward(&w, &bars, &run, &p).map(|r| (w.optimize, r)))
                });
            }
            Mode::MonteCarlo => {
                let Some(run) = self.strategies.current() else { return };
                let trades = run.broker.trades.clone();
                let cap = run.broker.props.initial_capital;
                let s = self.lab.setup.mc.clone();
                self.lab_start(move |_| Done::Mc(opentern_engine::quant::montecarlo::monte_carlo(&trades, cap, &s)));
            }
            Mode::Portfolio => self.lab_portfolio(target),
        }
    }

    fn lab_portfolio(&mut self, target: Target) {
        if target.build.is_none() {
            self.lab.status = "This strategy can't be compiled for other symbols.".into();
            return;
        }
        let symbols = match &self.lab.setup.universe {
            crate::screener::Universe::Watchlist(id) => self
                .watchlists
                .iter()
                .find(|w| w.id == *id)
                .map(|w| w.symbols())
                .unwrap_or_default(),
            crate::screener::Universe::Preset(name) => crate::screener::presets()
                .into_iter()
                .find(|(n, _)| n == name)
                .map(|(_, s)| s)
                .unwrap_or_default(),
        };
        if symbols.is_empty() {
            self.lab.status = "That list has no symbols.".into();
            return;
        }
        let capital = self
            .strategies
            .current()
            .map(|r| r.broker.props.initial_capital)
            .unwrap_or(1_000_000.0);
        self.lab.runs += 1;
        let seq_base = (1 << 41) + self.lab.runs * 10_000;
        let tf = target.tf;
        for (k, s) in symbols.iter().enumerate() {
            self.hub.send(opentern_data::Request::Fetch {
                seq: seq_base + k as u64,
                symbol: s.clone(),
                tf,
                count: self.lab.setup.port_bars.clamp(100, 20_000),
            });
        }
        self.lab.status = format!("Loading {} symbols…", symbols.len());
        self.lab.fetch = Some(PortFetch {
            seq_base,
            tf,
            want: symbols,
            got: Vec::new(),
            target,
            capital,
        });
    }

    /// Bars for a portfolio run arrived.
    pub(crate) fn lab_fetched(&mut self, seq: u64, id: &str, result: Result<Vec<Bar>, String>) {
        let Some(f) = &mut self.lab.fetch else { return };
        if seq < f.seq_base || seq >= f.seq_base + 10_000 {
            return;
        }
        let Some(sym) = f.want.iter().find(|s| s.id == id).cloned() else {
            return;
        };
        if f.got.iter().any(|(s, _)| s.id == id) {
            return;
        }
        f.got.push((sym, result));
        self.lab.status = format!("Loaded {} of {}…", f.got.len(), f.want.len());
        if f.got.len() < f.want.len() {
            return;
        }
        let Some(f) = self.lab.fetch.take() else { return };
        let n = f.got.len();
        self.lab_start(move |p| {
            p.add_total(n);
            let build = f.target.build.clone().expect("checked");
            let each = f.capital / n as f64;
            let extra = serde_json::json!({ "initial_capital": each });
            use rayon::prelude::*;
            let runs: Vec<(Symbol, LegRun)> = f
                .got
                .into_par_iter()
                .map(|(sym, bars)| {
                    let r = bars.and_then(|bars| {
                        if p.cancelled() {
                            return Err("Stopped.".into());
                        }
                        let (meta, factory) = build(&sym, &extra)?;
                        let tz = jiff::tz::TimeZone::get(&sym.timezone).unwrap_or(jiff::tz::TimeZone::UTC);
                        let ins = f.target.inputs.normalized(meta);
                        let b = run_study(meta, &ins, &factory, &bars, f.tf, &tz, None)
                            .ok_or("The script failed on this symbol.")?;
                        Ok((bars, b))
                    });
                    p.tick();
                    (sym, r)
                })
                .collect();
            let mut errors = Vec::new();
            let ok: Vec<(String, Vec<Bar>, Broker)> = runs
                .into_iter()
                .filter_map(|(s, r)| match r {
                    Ok((bars, b)) => Some((s.ticker().to_string(), bars, b)),
                    Err(e) => {
                        errors.push(format!("{}: {e}", s.ticker()));
                        None
                    }
                })
                .collect();
            if ok.is_empty() {
                return Done::Port(Err("No symbol could be tested.".into()));
            }
            let legs: Vec<portfolio::Leg> = ok
                .iter()
                .map(|(s, bars, b)| portfolio::Leg {
                    symbol: s.clone(),
                    bars,
                    broker: b,
                })
                .collect();
            Done::Port(Ok((portfolio::combine(&legs), errors)))
        });
    }

    /// For stills (`OPENTERN_STILL=lab:<mode>`): picks the mode, varies the
    /// first two inputs and runs as soon as the strategy has bars.
    pub(crate) fn lab_autorun(&mut self, mode: &str) {
        self.lab_load();
        self.lab.setup.mode = Mode::ALL
            .into_iter()
            .find(|m| {
                m.title()
                    .to_lowercase()
                    .replace('-', "")
                    .starts_with(&mode.to_lowercase().replace('-', ""))
            })
            .unwrap_or_default();
        if mode == "heatmap" {
            self.lab.view = OptView::Heatmap;
        }
        self.lab.autorun = true;
    }

    /// Picks up finished runs.
    pub(crate) fn poll_lab(&mut self) {
        self.lab_load();
        if self.lab.autorun
            && self.chart.bars.len() > 300
            && let Some(meta) = self.lab_target().ok().map(|t| t.meta)
        {
            self.lab.autorun = false;
            for (k, r) in self.lab_params(meta).iter_mut().enumerate() {
                r.on = k < 2;
            }
            self.lab_run();
        }
        let Some(job) = &self.lab.job else { return };
        let Ok(done) = job.rx.try_recv() else { return };
        let secs = job.started.elapsed().as_secs_f32();
        let cancelled = job.progress.cancelled();
        self.lab.job = None;
        let took = if secs < 10.0 {
            format!("{secs:.1} s")
        } else {
            format!("{secs:.0} s")
        };
        let stopped = if cancelled { "Stopped. " } else { "" };
        self.lab.status = match done {
            Done::Opt(Ok((s, o))) => {
                let msg = format!("{stopped}{} runs in {took}.", o.trials.len());
                let uid = self.lab.pending_uid;
                if s.params.len() >= 2 {
                    self.lab.heat = (0, 1);
                }
                self.lab.opt = Some((s, o, uid));
                msg
            }
            Done::Walk(Ok((s, r))) => {
                let msg = format!("{stopped}{} windows in {took}.", r.windows.len());
                self.lab.walk = Some((s, r));
                msg
            }
            Done::Mc(Ok(r)) => {
                let msg = format!("{} runs in {took}.", r.runs);
                self.lab.mc = Some(r);
                msg
            }
            Done::Port(Ok((r, errors))) => {
                let msg = if errors.is_empty() {
                    format!("{} symbols in {took}.", r.legs.len())
                } else {
                    format!("{} symbols in {took}; {} couldn't run.", r.legs.len(), errors.len())
                };
                self.lab.port = Some(r);
                self.lab.port_errors = errors;
                msg
            }
            Done::Opt(Err(e)) | Done::Walk(Err(e)) | Done::Mc(Err(e)) | Done::Port(Err(e)) => e,
        };
    }

    /// Puts a trial's inputs on the chart's strategy.
    fn lab_apply(&mut self, values: &[PValue]) {
        let Some((setup, _, uid)) = &self.lab.opt else { return };
        let Some(s) = self.chart.studies.iter().find(|s| s.uid == *uid) else {
            self.lab.status = "The strategy isn't on the chart any more.".into();
            return;
        };
        let ins = inputs_with(s.inst.meta, &s.inst.inputs, &setup.params, values);
        let uid = *uid;
        self.chart.set_inputs(uid, &ins);
        self.lab.status = "Applied to the chart.".into();
    }

    fn lab_export(&mut self, json: bool) {
        let mode = self.lab.setup.mode;
        let name = self.strategies.current().map(|r| r.name.clone()).unwrap_or_default();
        let text =
            match (mode, json) {
                (Mode::Optimize, false) => self.lab.opt.as_ref().map(|(s, o, _)| export::optimizer_csv(s, o)),
                (Mode::Optimize, true) => self.lab.opt.as_ref().and_then(|(s, o, _)| {
                    serde_json::to_string_pretty(&serde_json::json!({"setup": s, "result": o})).ok()
                }),
                (Mode::WalkForward, false) => self.lab.walk.as_ref().map(|(s, r)| export::walk_forward_csv(s, r)),
                (Mode::WalkForward, true) => self.lab.walk.as_ref().and_then(|(s, r)| {
                    serde_json::to_string_pretty(&serde_json::json!({"setup": s, "result": r})).ok()
                }),
                (Mode::MonteCarlo, false) => self.lab.mc.as_ref().map(export::monte_carlo_csv),
                (Mode::MonteCarlo, true) => self.lab.mc.as_ref().and_then(|r| serde_json::to_string_pretty(r).ok()),
                (Mode::Portfolio, false) => self.lab.port.as_ref().map(export::portfolio_csv),
                (Mode::Portfolio, true) => self
                    .lab
                    .port
                    .as_ref()
                    .and_then(|r| serde_json::to_string_pretty(r).ok()),
            };
        let Some(text) = text else { return };
        let dir = self.store.dir().join("exports");
        let ext = if json { "json" } else { "csv" };
        let path = dir.join(format!("{} {}.{ext}", safe_name(&name), mode.file()));
        let r = std::fs::create_dir_all(&dir).and_then(|_| std::fs::write(&path, text));
        self.lab.status = match r {
            Ok(()) => format!("Saved to {}", path.display()),
            Err(e) => format!("Couldn't save: {e}"),
        };
    }

    // ------------------------------------------------------- the agent

    pub(crate) fn lab_busy(&self) -> bool {
        self.lab.job.is_some() || self.lab.fetch.is_some()
    }

    pub(crate) fn lab_stop(&mut self) {
        if let Some(j) = &self.lab.job {
            j.progress.cancel.store(true, Ordering::Relaxed);
        }
        if self.lab.fetch.take().is_some() {
            self.lab.status = "Stopped.".into();
        }
    }

    /// The `strategy_lab` tool's set: changes the setup the way the tab's
    /// controls would.
    pub(crate) fn lab_configure(&mut self, a: &serde_json::Value) -> Result<(), String> {
        use crate::claude::control::norm;
        self.lab_load();
        let st = |k: &str| a.get(k).and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
        let n = |k: &str| a.get(k).and_then(|v| v.as_f64());
        let s = &mut self.lab.setup;
        if !st("mode").is_empty() {
            let want = norm(&st("mode"));
            s.mode = Mode::ALL
                .into_iter()
                .find(|m| norm(m.title()) == want || norm(&format!("{m:?}")) == want)
                .ok_or_else(|| {
                    format!(
                        "No lab mode \"{}\": optimize, walk_forward, monte_carlo or portfolio.",
                        st("mode")
                    )
                })?;
        }
        match norm(&st("method")).as_str() {
            "" => {}
            "grid" => s.method = MethodKind::Grid,
            "random" => s.method = MethodKind::Random,
            "bayesian" => s.method = MethodKind::Bayesian,
            m => return Err(format!("No method \"{m}\": grid, random or bayesian.")),
        }
        if !st("objective").is_empty() {
            let want = norm(&st("objective"));
            s.objective = Objective::ALL
                .into_iter()
                .find(|o| norm(&format!("{o:?}")) == want || norm(o.name()) == want)
                .ok_or_else(|| format!("No objective \"{}\".", st("objective")))?;
        }
        if let Some(v) = n("trials") {
            s.trials = v.clamp(1.0, 50_000.0) as usize;
        }
        if let Some(v) = n("min_trades") {
            s.min_trades = v.max(0.0) as usize;
        }
        if let Some(v) = n("in_sample") {
            s.in_sample = v.max(10.0) as usize;
        }
        if let Some(v) = n("out_sample") {
            s.out_sample = v.max(1.0) as usize;
        }
        if let Some(v) = a.get("anchored").and_then(|v| v.as_bool()) {
            s.anchored = v;
        }
        if let Some(v) = n("mc_runs") {
            s.mc.runs = v.clamp(10.0, 100_000.0) as usize;
        }
        match norm(&st("mc_method")).as_str() {
            "" => {}
            "bootstrap" => s.mc.method = Resample::Bootstrap,
            _ => s.mc.method = Resample::Shuffle,
        }
        if let Some(v) = n("ruin_pct") {
            s.mc.ruin_pct = v.clamp(1.0, 100.0);
        }
        if let Some(v) = n("bars") {
            s.port_bars = v.clamp(100.0, 20_000.0) as usize;
        }
        let u = st("universe");
        if !u.is_empty() {
            let list = self.watchlists.iter().find(|w| norm(&w.name) == norm(&u)).map(|w| w.id);
            let preset = crate::screener::presets()
                .into_iter()
                .map(|(p, _)| p)
                .find(|p| norm(p) == norm(&u));
            self.lab.setup.universe = match (list, preset) {
                (Some(id), _) => crate::screener::Universe::Watchlist(id),
                (None, Some(p)) => crate::screener::Universe::Preset(p.to_string()),
                _ => return Err(format!("No watchlist or preset called \"{u}\".")),
            };
        }
        if let Some(ps) = a.get("params").and_then(|v| v.as_array()) {
            let meta = self.lab_target()?.meta;
            let rows = self.lab_params(meta);
            // Listed inputs are switched on unless they say otherwise.
            for p in ps {
                let name = p.get("input").or(p.get("id")).and_then(|v| v.as_str()).unwrap_or("");
                let row = rows
                    .iter_mut()
                    .find(|r| r.id == name || norm(&r.title) == norm(name))
                    .ok_or_else(|| format!("The strategy has no input \"{name}\" the lab can vary."))?;
                row.on = p.get("on").and_then(|v| v.as_bool()).unwrap_or(true);
                let f = |k: &str| p.get(k).and_then(|v| v.as_f64());
                match &mut row.range {
                    Range::Int { from, to, step } => {
                        if let Some(v) = f("from") {
                            *from = v.round() as i64;
                        }
                        if let Some(v) = f("to") {
                            *to = v.round() as i64;
                        }
                        if let Some(v) = f("step") {
                            *step = (v.round() as i64).max(1);
                        }
                    }
                    Range::Float { from, to, step } => {
                        if let Some(v) = f("from") {
                            *from = v;
                        }
                        if let Some(v) = f("to") {
                            *to = v;
                        }
                        if let Some(v) = f("step") {
                            *step = v.abs();
                        }
                    }
                    _ => {}
                }
            }
        }
        self.lab_save();
        Ok(())
    }

    /// The setup and the strategy's inputs, for the agent.
    pub(crate) fn lab_describe(&mut self) -> serde_json::Value {
        use serde_json::json;
        self.lab_load();
        let params = match self.lab_target() {
            Ok(t) => json!(self.lab_params(t.meta).clone()),
            Err(e) => json!(e),
        };
        let s = &self.lab.setup;
        let out = json!({
            "strategy": self.strategies.current().map(|r| r.name.clone()),
            "mode": format!("{:?}", s.mode),
            "method": format!("{:?}", s.method),
            "trials": s.trials,
            "objective": format!("{:?}", s.objective),
            "min_trades": s.min_trades,
            "in_sample": s.in_sample,
            "out_sample": s.out_sample,
            "anchored": s.anchored,
            "monte_carlo": s.mc,
            "portfolio_universe": format!("{:?}", s.universe),
            "portfolio_bars": s.port_bars,
            "params": params,
            "status": self.lab.status,
        });
        json!({"content": [{"type": "text", "text": serde_json::to_string_pretty(&out).unwrap_or_default()}]})
    }

    /// The latest results of the current mode, trimmed for the agent.
    pub(crate) fn lab_results(&self) -> serde_json::Value {
        use serde_json::json;
        let titled = |s: &Setup, values: &[PValue]| -> serde_json::Map<String, serde_json::Value> {
            s.params
                .iter()
                .zip(values)
                .map(|(p, v)| (p.title.clone(), json!(v.to_string())))
                .collect()
        };
        let out = match self.lab.setup.mode {
            Mode::Optimize => self.lab.opt.as_ref().map(|(s, o, _)| {
                json!({
                    "runs": o.trials.len(),
                    "objective": s.objective.name(),
                    "best_first": o.trials.iter().take(10).enumerate().map(|(k, t)| json!({
                        "rank": k + 1,
                        "inputs": titled(s, &t.values),
                        "score": if t.score.is_nan() { json!(null) } else { json!(t.score) },
                        "summary": t.summary,
                    })).collect::<Vec<_>>(),
                })
            }),
            Mode::WalkForward => self.lab.walk.as_ref().map(|(s, r)| {
                json!({
                    "windows": r.windows.iter().map(|w| json!({
                        "in_sample": [crate::claude::control::iso(w.in_time.0), crate::claude::control::iso(w.in_time.1)],
                        "out_of_sample": [crate::claude::control::iso(w.out_time.0), crate::claude::control::iso(w.out_time.1)],
                        "best_inputs": w.values.as_ref().map(|v| titled(s, v)),
                        "out_net": w.out_net,
                        "out_trades": w.out_trades,
                    })).collect::<Vec<_>>(),
                    "out_of_sample_net": r.out_net,
                    "out_of_sample_trades": r.out_trades,
                    "out_of_sample_percent_profitable": r.out_percent_profitable,
                    "out_of_sample_max_drawdown": r.out_max_drawdown,
                    "efficiency_pct": r.efficiency,
                    "profitable_windows_pct": r.profitable_windows,
                })
            }),
            Mode::MonteCarlo => self.lab.mc.as_ref().map(|r| {
                json!({
                    "runs": r.runs,
                    "trades": r.trades,
                    "actual_net": r.actual_net,
                    "actual_max_drawdown": r.actual_max_drawdown,
                    "net_profit_percentiles": r.net_profit,
                    "max_drawdown_percentiles": r.max_drawdown,
                    "max_drawdown_pct_percentiles": r.max_drawdown_pct,
                    "longest_losing_streak_percentiles": r.longest_losing_streak,
                    "chance_of_loss_pct": r.prob_loss,
                    "chance_of_ruin_pct": r.prob_ruin,
                })
            }),
            Mode::Portfolio => self.lab.port.as_ref().map(|r| {
                json!({
                    "legs": r.legs,
                    "net_profit": r.net_profit,
                    "net_profit_pct": r.net_profit_pct,
                    "total_trades": r.total_trades,
                    "percent_profitable": r.percent_profitable,
                    "profit_factor": r.profit_factor,
                    "max_drawdown": r.max_drawdown,
                    "max_drawdown_pct": r.max_drawdown_pct,
                    "sharpe": r.sharpe,
                    "cagr": r.cagr,
                    "buy_hold_pct": r.buy_hold_pct,
                    "errors": self.lab.port_errors,
                })
            }),
        };
        let mut o = out.unwrap_or_else(|| json!({"results": null}));
        o["mode"] = json!(self.lab.setup.mode.title());
        o["status"] = json!(self.lab.status);
        json!({"content": [{"type": "text", "text": serde_json::to_string_pretty(&o).unwrap_or_default()}]})
    }

    /// Puts the optimizer's `rank`th result on the chart.
    pub(crate) fn lab_apply_rank(&mut self, rank: usize) -> Result<String, String> {
        let (values, title) = {
            let (s, o, _) = self.lab.opt.as_ref().ok_or("Run the optimizer first.")?;
            let t = o
                .trials
                .get(rank - 1)
                .filter(|t| !t.score.is_nan())
                .ok_or("There's no scored result at that rank.")?;
            let title: Vec<String> = s
                .params
                .iter()
                .zip(&t.values)
                .map(|(p, v)| format!("{} {v}", p.title))
                .collect();
            (t.values.clone(), title.join(", "))
        };
        self.lab_apply(&values);
        if self.lab.status == "Applied to the chart." {
            Ok(format!("Applied {title} to the strategy on the chart."))
        } else {
            Err(self.lab.status.clone())
        }
    }

    // ---------------------------------------------------------------- UI

    pub(crate) fn lab_tab(&mut self, ui: &mut Ui, t: &Theme) {
        self.lab_load();
        let running = self.lab.job.is_some() || self.lab.fetch.is_some();
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            for m in Mode::ALL {
                if small_button(ui, t, m.title(), self.lab.setup.mode == m).clicked() {
                    self.lab.setup.mode = m;
                }
            }
            ui.add_space(10.0);
            if let Some(r) = self.strategies.current()
                && ui.available_width() > 760.0
            {
                ui.label(egui::RichText::new(&r.name).font(theme::semibold(13.0)).color(t.text2));
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                if running {
                    if small_button(ui, t, "Stop", false).clicked() {
                        if let Some(j) = &self.lab.job {
                            j.progress.cancel.store(true, Ordering::Relaxed);
                        }
                        if self.lab.fetch.take().is_some() {
                            self.lab.status = "Stopped.".into();
                        }
                    }
                    let frac = self.lab.job.as_ref().map(|j| j.progress.fraction()).unwrap_or(0.0);
                    let (r, _) = ui.allocate_exact_size(vec2(120.0, 6.0), Sense::hover());
                    let p = ui.painter();
                    p.rect_filled(r, CornerRadius::same(3), t.hover_wash());
                    p.rect_filled(
                        Rect::from_min_size(r.min, vec2(r.width() * frac, r.height())),
                        CornerRadius::same(3),
                        t.accent,
                    );
                    ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
                } else {
                    if crate::widgets::button(ui, crate::widgets::Kind::Primary, true, Some(ph::PLAY), "Run").clicked()
                    {
                        self.lab_run();
                    }
                    let has = match self.lab.setup.mode {
                        Mode::Optimize => self.lab.opt.is_some(),
                        Mode::WalkForward => self.lab.walk.is_some(),
                        Mode::MonteCarlo => self.lab.mc.is_some(),
                        Mode::Portfolio => self.lab.port.is_some(),
                    };
                    if has {
                        if small_button(ui, t, "JSON", false)
                            .on_hover_text("Save the results as JSON")
                            .clicked()
                        {
                            self.lab_export(true);
                        }
                        if small_button(ui, t, "CSV", false)
                            .on_hover_text("Save the results as CSV")
                            .clicked()
                        {
                            self.lab_export(false);
                        }
                    }
                }
                ui.add(egui::Label::new(egui::RichText::new(&self.lab.status).color(t.text3).size(11.5)).truncate());
            });
        });
        ui.add_space(6.0);
        let target = self.lab_target();
        let meta = target.as_ref().ok().map(|t| t.meta);
        if self.strategies.runs.is_empty() {
            crate::widgets::empty_state(ui, t, ph::FLASK, "No strategy on the chart");
            return;
        }
        let full = ui.available_rect_before_wrap();
        let setup_w = (full.width() * 0.36).clamp(280.0, 380.0);
        let left = Rect::from_min_size(full.min, vec2(setup_w, full.height()));
        let right = Rect::from_min_max(pos2(left.right() + 14.0, full.top()), full.max);
        ui.painter()
            .vline(left.right() + 7.0, full.y_range(), Stroke::new(1.0, t.border));
        let mut lu = ui.new_child(egui::UiBuilder::new().max_rect(left));
        lu.set_clip_rect(left.intersect(ui.clip_rect()));
        egui::ScrollArea::vertical().id_salt("lab-setup").show(&mut lu, |ui| {
            ui.set_width(setup_w - 12.0);
            ui.add_enabled_ui(!running, |ui| self.lab_setup_ui(ui, t, meta, &target));
        });
        let mut ru = ui.new_child(egui::UiBuilder::new().max_rect(right));
        ru.set_clip_rect(right.intersect(ui.clip_rect()));
        self.lab.view_h = right.height();
        match self.lab.setup.mode {
            // The optimizer's list scrolls itself; the others scroll as a whole.
            Mode::Optimize => self.lab_opt_results(&mut ru, t),
            mode => {
                egui::ScrollArea::vertical()
                    .id_salt("lab-results")
                    .auto_shrink([false, false])
                    .show(&mut ru, |ui| match mode {
                        Mode::WalkForward => self.lab_walk_results(ui, t),
                        Mode::MonteCarlo => self.lab_mc_results(ui, t),
                        _ => self.lab_port_results(ui, t),
                    });
            }
        }
        ui.allocate_rect(full, Sense::hover());
    }

    /// Charts take about half the results area.
    fn chart_h(&self) -> f32 {
        (self.lab.view_h * 0.55).clamp(150.0, 340.0)
    }

    fn fmt(&self) -> Fmt {
        Fmt {
            ccy: if self.chart.symbol.currency.is_empty() {
                "USD".into()
            } else {
                self.chart.symbol.currency.clone()
            },
            tz: self.chart.tz.clone(),
        }
    }

    fn lab_setup_ui(&mut self, ui: &mut Ui, t: &Theme, meta: Option<&'static Meta>, target: &Result<Target, String>) {
        let mode = self.lab.setup.mode;
        if let Err(e) = target
            && mode != Mode::MonteCarlo
        {
            ui.label(egui::RichText::new(e).color(t.text3));
            return;
        }
        match mode {
            Mode::Optimize | Mode::WalkForward => {
                let Some(meta) = meta else { return };
                self.param_table(ui, t, meta);
                ui.add_space(8.0);
                let s = &mut self.lab.setup;
                row(ui, t, "Maximize", |ui| {
                    crate::widgets::combo("lab-objective")
                        .selected_text(s.objective.name())
                        .width(150.0)
                        .show_ui(ui, |ui| {
                            for o in Objective::ALL {
                                crate::widgets::option_value(ui, &mut s.objective, o, o.name());
                            }
                        });
                });
                row(ui, t, "Search", |ui| {
                    crate::widgets::combo("lab-method")
                        .selected_text(match s.method {
                            MethodKind::Grid => "Every combination",
                            MethodKind::Random => "Random",
                            MethodKind::Bayesian => "Bayesian",
                        })
                        .width(150.0)
                        .show_ui(ui, |ui| {
                            crate::widgets::option_value(ui, &mut s.method, MethodKind::Grid, "Every combination");
                            crate::widgets::option_value(ui, &mut s.method, MethodKind::Random, "Random");
                            crate::widgets::option_value(ui, &mut s.method, MethodKind::Bayesian, "Bayesian");
                        });
                });
                if s.method != MethodKind::Grid {
                    row(ui, t, "Runs", |ui| {
                        ui.add(egui::DragValue::new(&mut s.trials).range(10..=5000));
                    });
                }
                row(ui, t, "Fewest trades", |ui| {
                    ui.add(egui::DragValue::new(&mut s.min_trades).range(0..=10_000));
                });
                if mode == Mode::WalkForward {
                    row(ui, t, "In-sample bars", |ui| {
                        ui.add(egui::DragValue::new(&mut s.in_sample).range(20..=1_000_000).speed(5));
                    });
                    row(ui, t, "Out-of-sample bars", |ui| {
                        ui.add(egui::DragValue::new(&mut s.out_sample).range(5..=1_000_000).speed(2));
                    });
                    row(ui, t, "Anchored", |ui| {
                        crate::widgets::checkbox(ui, &mut s.anchored, "")
                            .on_hover_text("Every in-sample window starts at the first bar");
                    });
                }
                let setup = self.lab_opt_setup(meta);
                let combos = optimize::combinations(&setup.params);
                let per = optimize::planned(&setup);
                let n = self.chart.bars.len();
                let text = match (setup.params.len(), per) {
                    (0, _) => "Tick the inputs to vary.".to_string(),
                    (_, None) => {
                        format!("{combos} combinations: too many for every combination. Use Random or Bayesian.")
                    }
                    (_, Some(p)) if mode == Mode::WalkForward => {
                        let w = walkforward::windows(
                            n,
                            &WalkSetup {
                                optimize: setup.clone(),
                                in_sample: self.lab.setup.in_sample,
                                out_sample: self.lab.setup.out_sample,
                                anchored: self.lab.setup.anchored,
                            },
                        )
                        .len();
                        format!("{w} windows × {p} runs on the chart's {n} bars.")
                    }
                    (_, Some(p)) => format!("{p} runs of {combos} combinations on the chart's {n} bars."),
                };
                ui.add_space(6.0);
                ui.label(egui::RichText::new(text).color(t.text3).size(11.5));
            }
            Mode::MonteCarlo => {
                let s = &mut self.lab.setup.mc;
                row(ui, t, "Runs", |ui| {
                    ui.add(egui::DragValue::new(&mut s.runs).range(100..=100_000).speed(10));
                });
                row(ui, t, "Method", |ui| {
                    crate::widgets::combo("lab-mc")
                        .selected_text(match s.method {
                            Resample::Shuffle => "Shuffle the order",
                            Resample::Bootstrap => "Draw with replacement",
                        })
                        .width(150.0)
                        .show_ui(ui, |ui| {
                            crate::widgets::option_value(ui, &mut s.method, Resample::Shuffle, "Shuffle the order");
                            crate::widgets::option_value(
                                ui,
                                &mut s.method,
                                Resample::Bootstrap,
                                "Draw with replacement",
                            );
                        });
                });
                row(ui, t, "Ruin at drawdown", |ui| {
                    ui.add(egui::DragValue::new(&mut s.ruin_pct).range(1.0..=100.0).suffix(" %"));
                });
            }
            Mode::Portfolio => {
                let current = match &self.lab.setup.universe {
                    crate::screener::Universe::Watchlist(id) => self
                        .watchlists
                        .iter()
                        .find(|w| w.id == *id)
                        .map(|w| w.name.clone())
                        .unwrap_or_else(|| "Watchlist".into()),
                    crate::screener::Universe::Preset(n) => n.clone(),
                };
                let lists = self.watchlists.clone();
                let s = &mut self.lab.setup;
                row(ui, t, "Symbols", |ui| {
                    crate::widgets::combo("lab-universe")
                        .selected_text(current)
                        .width(150.0)
                        .show_ui(ui, |ui| {
                            for (name, _) in crate::screener::presets() {
                                let u = crate::screener::Universe::Preset(name.into());
                                if crate::widgets::option(ui, s.universe == u, name).clicked() {
                                    s.universe = u;
                                }
                            }
                            ui.separator();
                            for wl in &lists {
                                let u = crate::screener::Universe::Watchlist(wl.id);
                                if crate::widgets::option(
                                    ui,
                                    s.universe == u,
                                    format!("{}  {}", ph::LIST_BULLETS, wl.name),
                                )
                                .clicked()
                                {
                                    s.universe = u;
                                }
                            }
                        });
                });
                row(ui, t, "Bars per symbol", |ui| {
                    ui.add(egui::DragValue::new(&mut s.port_bars).range(100..=20_000).speed(10));
                });
                ui.add_space(6.0);
            }
        }
    }

    fn param_table(&mut self, ui: &mut Ui, t: &Theme, meta: &'static Meta) {
        ui.label(
            egui::RichText::new("INPUTS TO VARY")
                .font(theme::semibold(11.0))
                .color(t.text3),
        );
        ui.add_space(2.0);
        let rows = self.lab_params(meta);
        if rows.is_empty() {
            ui.label(egui::RichText::new("This strategy has no inputs to vary.").color(t.text3));
            return;
        }
        for r in rows.iter_mut() {
            let title = egui::RichText::new(&r.title).color(if r.on { t.text } else { t.text2 });
            crate::widgets::checkbox(ui, &mut r.on, title);
            if !r.on {
                continue;
            }
            // The range on its own line under the name, so it fits narrow panels.
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                ui.add_space(24.0);
                let word = |ui: &mut Ui, w: &str| {
                    ui.label(egui::RichText::new(w).color(t.text3));
                };
                match &mut r.range {
                    Range::Int { from, to, step } => {
                        word(ui, "from");
                        ui.add(egui::DragValue::new(from).speed(0.2));
                        word(ui, "to");
                        ui.add(egui::DragValue::new(to).speed(0.2));
                        word(ui, "by");
                        ui.add(egui::DragValue::new(step).range(1..=1_000_000).speed(0.1));
                    }
                    Range::Float { from, to, step } => {
                        word(ui, "from");
                        ui.add(egui::DragValue::new(from).speed(0.01).max_decimals(6));
                        word(ui, "to");
                        ui.add(egui::DragValue::new(to).speed(0.01).max_decimals(6));
                        word(ui, "by");
                        ui.add(
                            egui::DragValue::new(step)
                                .range(1e-9..=1e9)
                                .speed(0.001)
                                .max_decimals(6),
                        );
                    }
                    Range::Bool => word(ui, "Tries on and off"),
                    Range::Choice(c) => word(ui, &format!("Tries all {} choices", c.len())),
                }
            });
            ui.add_space(2.0);
        }
    }

    // ---- results

    fn lab_opt_results(&mut self, ui: &mut Ui, t: &Theme) {
        let Some((setup, outcome, _)) = &self.lab.opt else {
            return;
        };
        let setup = setup.clone();
        let mut apply: Option<Vec<PValue>> = None;
        // Headline: the best run, and how to look at the rest.
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            if setup.params.len() >= 2 {
                if small_button(ui, t, "List", self.lab.view == OptView::List).clicked() {
                    self.lab.view = OptView::List;
                }
                if small_button(ui, t, "Heatmap", self.lab.view == OptView::Heatmap).clicked() {
                    self.lab.view = OptView::Heatmap;
                }
                ui.add_space(8.0);
            }
            match outcome.best() {
                Some(b) => {
                    let vals: Vec<String> = setup
                        .params
                        .iter()
                        .zip(&b.values)
                        .map(|(p, v)| format!("{} {v}", p.title))
                        .collect();
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if small_button(ui, t, "Apply best", false)
                            .on_hover_text("Put the best inputs on the chart's strategy")
                            .clicked()
                        {
                            apply = Some(b.values.clone());
                        }
                        ui.add_space(6.0);
                        let text = format!(
                            "Best: {}  →  {} ({}), {} trades",
                            vals.join(" · "),
                            crate::strategy::grouped(b.summary.net_profit, 2),
                            pct(b.summary.net_profit_pct),
                            b.summary.total_trades
                        );
                        ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                            ui.add(egui::Label::new(egui::RichText::new(&text).color(t.text2)).truncate())
                                .on_hover_text(&text);
                        });
                    });
                }
                None => {
                    ui.label(
                        egui::RichText::new("No run met the conditions (too few trades, or the script failed).")
                            .color(t.text3),
                    );
                }
            }
        });
        ui.add_space(6.0);
        let view = if setup.params.len() >= 2 {
            self.lab.view
        } else {
            OptView::List
        };
        match view {
            OptView::List => {
                if let Some(v) = self.opt_list(ui, t, &setup) {
                    apply = Some(v);
                }
            }
            OptView::Heatmap => {
                if let Some(v) = self.opt_heatmap(ui, t, &setup) {
                    apply = Some(v);
                }
            }
        }
        if let Some(v) = apply {
            self.lab_apply(&v);
        }
    }

    /// The trials as a table; a click selects, a double-click applies.
    fn opt_list(&mut self, ui: &mut Ui, t: &Theme, setup: &Setup) -> Option<Vec<PValue>> {
        let (_, outcome, _) = self.lab.opt.as_ref()?;
        let mut apply = None;
        let score_col = setup.objective != Objective::NetProfit;
        let heads: Vec<String> = std::iter::once("#".to_string())
            .chain(setup.params.iter().map(|p| p.title.clone()))
            .chain(score_col.then(|| setup.objective.name().to_string()))
            .chain(
                ["Net P&L", "Trades", "Win %", "Profit factor", "Max DD"]
                    .iter()
                    .map(|s| s.to_string()),
            )
            .collect();
        let w = ui.available_width();
        // "#" is narrow; the rest share the width (and scroll when tight).
        let num_w = 36.0;
        let col_w = ((w - num_w) / (heads.len() - 1) as f32).clamp(92.0, 150.0);
        let row_h = 24.0;
        let table_w = num_w + col_w * (heads.len() - 1) as f32;
        let x_at = |k: usize| {
            if k == 0 {
                num_w - 8.0
            } else {
                num_w + col_w * k as f32 - 8.0
            }
        };
        egui::ScrollArea::horizontal().id_salt("lab-opt-h").show(ui, |ui| {
            ui.set_min_width(table_w);
            let (r, _) = ui.allocate_exact_size(vec2(table_w, row_h), Sense::hover());
            for (k, h) in heads.iter().enumerate() {
                ui.painter().text(
                    pos2(r.left() + x_at(k), r.center().y),
                    Align2::RIGHT_CENTER,
                    h,
                    theme::medium(12.0),
                    t.text3,
                );
            }
            ui.painter().hline(r.x_range(), r.bottom(), Stroke::new(1.0, t.border));
            egui::ScrollArea::vertical()
                .id_salt("lab-opt-rows")
                .auto_shrink([false, false])
                .show_rows(ui, row_h, outcome.trials.len(), |ui, range| {
                    for i in range {
                        let tr = &outcome.trials[i];
                        let (r, resp) = ui.allocate_exact_size(vec2(table_w, row_h), Sense::click());
                        let p = ui.painter();
                        if self.lab.selected == Some(i) {
                            p.rect_filled(r, CornerRadius::same(4), t.accent_soft());
                        } else if resp.hovered() {
                            p.rect_filled(r, CornerRadius::same(4), t.hover_wash());
                        }
                        let s = &tr.summary;
                        let mut cells: Vec<(String, Color32)> = vec![((i + 1).to_string(), t.text3)];
                        cells.extend(tr.values.iter().map(|v| (v.to_string(), t.text)));
                        if tr.failed {
                            cells.push(("Script failed".into(), t.text3));
                        } else {
                            if score_col {
                                cells.push((num(tr.score, 2), t.text));
                            }
                            cells.extend([
                                (crate::strategy::grouped(s.net_profit, 2), tone(t, s.net_profit)),
                                (s.total_trades.to_string(), t.text),
                                (pct(s.percent_profitable), t.text),
                                (num(s.profit_factor, 3), t.text),
                                (crate::strategy::grouped(s.max_drawdown, 2), t.text),
                            ]);
                        }
                        for (k, (c, col)) in cells.iter().enumerate() {
                            p.text(
                                pos2(r.left() + x_at(k), r.center().y),
                                Align2::RIGHT_CENTER,
                                c,
                                theme::regular(12.5),
                                *col,
                            );
                        }
                        if resp.double_clicked() {
                            apply = Some(tr.values.clone());
                        } else if resp.clicked() {
                            self.lab.selected = Some(i);
                        }
                        resp.on_hover_text("Double-click to put these inputs on the chart");
                    }
                });
        });
        apply
    }

    fn opt_heatmap(&mut self, ui: &mut Ui, t: &Theme, setup: &Setup) -> Option<Vec<PValue>> {
        let n = setup.params.len();
        let (mut px, mut py) = self.lab.heat;
        px = px.min(n - 1);
        py = py.min(n - 1);
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Across").color(t.text3));
            crate::widgets::combo("heat-x")
                .selected_text(&setup.params[px].title)
                .show_ui(ui, |ui| {
                    for (k, p) in setup.params.iter().enumerate() {
                        crate::widgets::option_value(ui, &mut px, k, &p.title);
                    }
                });
            ui.label(egui::RichText::new("down").color(t.text3));
            crate::widgets::combo("heat-y")
                .selected_text(&setup.params[py].title)
                .show_ui(ui, |ui| {
                    for (k, p) in setup.params.iter().enumerate() {
                        crate::widgets::option_value(ui, &mut py, k, &p.title);
                    }
                });
        });
        self.lab.heat = (px, py);
        if px == py {
            ui.label(egui::RichText::new("Pick two different inputs.").color(t.text3));
            return None;
        }
        let (_, outcome, _) = self.lab.opt.as_ref()?;
        let heat = optimize::heatmap(setup, &outcome.trials, px, py)?;
        let area = ui.available_rect_before_wrap();
        let (left, bottom) = (64.0, 22.0);
        let grid = Rect::from_min_max(
            pos2(area.left() + left, area.top() + 4.0),
            pos2(area.right() - 8.0, area.bottom() - bottom),
        );
        if grid.width() < 40.0 || grid.height() < 40.0 {
            return None;
        }
        let resp = ui.allocate_rect(area, Sense::click());
        let (cols, rows) = (heat.xs.len(), heat.ys.len());
        let cw = grid.width() / cols as f32;
        let ch = grid.height() / rows as f32;
        let vals: Vec<f64> = heat.cells.iter().flatten().cloned().filter(|v| !v.is_nan()).collect();
        let (lo, hi) = vals
            .iter()
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), v| (a.min(*v), b.max(*v)));
        let p = ui.painter_at(area);
        let hover = resp.hover_pos();
        let mut hovered = None;
        for (y, row) in heat.cells.iter().enumerate() {
            for (x, v) in row.iter().enumerate() {
                // Highest values at the top.
                let r = Rect::from_min_size(
                    pos2(grid.left() + cw * x as f32, grid.bottom() - ch * (y + 1) as f32),
                    vec2(cw, ch),
                )
                .shrink(if cw > 6.0 && ch > 6.0 { 0.5 } else { 0.0 });
                let c = if v.is_nan() {
                    t.hover_wash()
                } else {
                    let k = if hi > lo { ((v - lo) / (hi - lo)) as f32 } else { 1.0 };
                    heat_color(t, k)
                };
                p.rect_filled(r, CornerRadius::ZERO, c);
                if hover.is_some_and(|h| r.contains(h)) {
                    hovered = Some((x, y, *v));
                    p.rect_stroke(
                        r,
                        CornerRadius::ZERO,
                        Stroke::new(1.5, t.text),
                        egui::StrokeKind::Inside,
                    );
                }
                if cw > 46.0 && ch > 16.0 && !v.is_nan() {
                    p.text(
                        r.center(),
                        Align2::CENTER_CENTER,
                        num(*v, 1),
                        theme::regular(10.5),
                        t.text,
                    );
                }
            }
        }
        // Axis labels, thinned to fit.
        let every_x = ((40.0 / cw).ceil() as usize).max(1);
        for (x, v) in heat.xs.iter().enumerate().step_by(every_x) {
            p.text(
                pos2(grid.left() + cw * (x as f32 + 0.5), grid.bottom() + 4.0),
                Align2::CENTER_TOP,
                v.to_string(),
                theme::regular(11.0),
                t.text3,
            );
        }
        let every_y = ((16.0 / ch).ceil() as usize).max(1);
        for (y, v) in heat.ys.iter().enumerate().step_by(every_y) {
            p.text(
                pos2(grid.left() - 6.0, grid.bottom() - ch * (y as f32 + 0.5)),
                Align2::RIGHT_CENTER,
                v.to_string(),
                theme::regular(11.0),
                t.text3,
            );
        }
        let mut apply = None;
        if let Some((x, y, v)) = hovered {
            let tip = format!(
                "{} {} · {} {}\n{}: {}",
                setup.params[px].title,
                heat.xs[x],
                setup.params[py].title,
                heat.ys[y],
                setup.objective.name(),
                if v.is_nan() { "not run".into() } else { num(v, 2) }
            );
            let resp = resp.on_hover_text(format!("{tip}\nClick to put the best of these on the chart"));
            if resp.clicked() && !v.is_nan() {
                apply = outcome
                    .trials
                    .iter()
                    .find(|tr| tr.values.get(px) == Some(&heat.xs[x]) && tr.values.get(py) == Some(&heat.ys[y]))
                    .map(|tr| tr.values.clone());
            }
        }
        apply
    }

    fn lab_walk_results(&mut self, ui: &mut Ui, t: &Theme) {
        let Some((setup, r)) = &self.lab.walk else {
            return;
        };
        let f = self.fmt();
        let cards = [
            ("Out-of-sample P&L", f.money(r.out_net), tone(t, r.out_net)),
            ("Efficiency", pct(r.efficiency), t.text),
            ("Profitable windows", pct(r.profitable_windows), t.text),
            ("Trades", r.out_trades.to_string(), t.text),
            ("Max drawdown", f.money(r.out_max_drawdown), t.text),
        ];
        cards_row(ui, t, &cards);
        let tips = "Efficiency: out-of-sample profit per bar as a share of in-sample profit per bar. Above 50% is usually read as robust.";
        ui.label(egui::RichText::new(tips).color(t.text3).size(11.5));
        ui.add_space(4.0);
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), self.chart_h()), Sense::hover());
        let xs: Vec<f64> = r.equity.iter().map(|e| e.0 as f64).collect();
        let ys: Vec<f64> = r.equity.iter().map(|e| e.1).collect();
        curve_chart(
            ui,
            t,
            rect,
            &xs,
            &[(&ys, t.accent)],
            None,
            r.initial_capital,
            resp.hover_pos(),
            &|x| time_text(x as i64, &f.tz),
        );
        ui.add_space(6.0);
        let mut heads = vec!["Out of sample".to_string()];
        heads.extend(setup.params.iter().map(|p| p.title.clone()));
        heads.extend(["In-sample P&L", "Out-of-sample P&L", "Trades"].map(String::from));
        let rows: Vec<Vec<(String, Color32)>> = r
            .windows
            .iter()
            .map(|w| {
                let mut v = vec![(
                    format!("{} — {}", date(w.out_time.0, &f.tz), date(w.out_time.1, &f.tz)),
                    t.text2,
                )];
                for i in 0..setup.params.len() {
                    let s = w.values.as_ref().and_then(|v| v.get(i)).map(|v| v.to_string());
                    v.push((s.unwrap_or_else(|| "—".into()), t.text));
                }
                v.push((f.money(w.in_sample.net_profit), tone(t, w.in_sample.net_profit)));
                v.push((f.money(w.out_net), tone(t, w.out_net)));
                v.push((w.out_trades.to_string(), t.text));
                v
            })
            .collect();
        grid_table(ui, t, "lab-walk", &heads, &rows);
    }

    fn lab_mc_results(&mut self, ui: &mut Ui, t: &Theme) {
        let Some(r) = &self.lab.mc else {
            return;
        };
        let f = self.fmt();
        // Shuffling never changes the total, so its cards are about drawdowns.
        let cards = if self.lab.setup.mc.method == Resample::Shuffle {
            vec![
                (
                    "Net P&L (every run)",
                    f.money(r.net_profit.p50),
                    tone(t, r.net_profit.p50),
                ),
                ("Backtest's max drawdown", f.money(r.actual_max_drawdown), t.text),
                ("Median max drawdown", pct(r.max_drawdown_pct.p50), t.text),
                ("Max drawdown, worst 5%", pct(r.max_drawdown_pct.p95), t.text),
                ("Losing streak, worst 5%", num(r.longest_losing_streak.p95, 0), t.text),
                ("Chance of ruin", pct(r.prob_ruin), t.text),
            ]
        } else {
            vec![
                ("Median P&L", f.money(r.net_profit.p50), tone(t, r.net_profit.p50)),
                ("P&L, worst 5%", f.money(r.net_profit.p5), tone(t, r.net_profit.p5)),
                ("Median max drawdown", pct(r.max_drawdown_pct.p50), t.text),
                ("Max drawdown, worst 5%", pct(r.max_drawdown_pct.p95), t.text),
                ("Chance of a loss", pct(r.prob_loss), t.text),
                ("Chance of ruin", pct(r.prob_ruin), t.text),
            ]
        };
        cards_row(ui, t, &cards);
        ui.add_space(4.0);
        let w = ui.available_width();
        let side = w >= 700.0;
        let table_w = if side { 380.0f32.min(w * 0.45) } else { w };
        let chart_w = if side { w - table_w - 12.0 } else { w };
        let (rect, resp) = ui.allocate_exact_size(vec2(chart_w, self.chart_h()), Sense::hover());
        let xs: Vec<f64> = (0..r.bands.len()).map(|i| i as f64).collect();
        let lo: Vec<f64> = r.bands.iter().map(|b| b.0).collect();
        let mid: Vec<f64> = r.bands.iter().map(|b| b.1).collect();
        let hi: Vec<f64> = r.bands.iter().map(|b| b.2).collect();
        curve_chart(
            ui,
            t,
            rect,
            &xs,
            &[(&mid, t.accent)],
            Some((&lo, &hi)),
            r.initial_capital,
            resp.hover_pos(),
            &|x| format!("After trade {}", x as usize),
        );
        let right = if side {
            Rect::from_min_max(
                pos2(rect.right() + 12.0, rect.top()),
                pos2(rect.right() + 12.0 + table_w, rect.bottom()),
            )
        } else {
            ui.add_space(6.0);
            let (r, _) = ui.allocate_exact_size(vec2(w, 28.0 * 5.0 + 4.0), Sense::hover());
            r
        };
        let mut cu = ui.new_child(egui::UiBuilder::new().max_rect(right));
        let rows = [
            ("Net profit", &r.net_profit, true),
            ("Max drawdown", &r.max_drawdown, true),
            ("Max drawdown %", &r.max_drawdown_pct, false),
            ("Losing streak", &r.longest_losing_streak, false),
        ];
        let rows: Vec<(&str, Vec<(String, Color32)>)> = rows
            .iter()
            .map(|(name, s, money)| {
                let v = |x: f64| {
                    if *money {
                        (crate::strategy::grouped(x, 0), t.text)
                    } else if name.ends_with('%') {
                        (pct(x), t.text)
                    } else {
                        (num(x, 0), t.text)
                    }
                };
                (*name, vec![v(s.p5), v(s.p50), v(s.p95)])
            })
            .collect();
        table(&mut cu, t, "lab-mc", &["5th", "Median", "95th"], &rows);
    }

    fn lab_port_results(&mut self, ui: &mut Ui, t: &Theme) {
        let Some(r) = &self.lab.port else {
            return;
        };
        let f = self.fmt();
        let cards = [
            ("Total P&L", f.money(r.net_profit), tone(t, r.net_profit)),
            ("Return", pct(r.net_profit_pct), tone(t, r.net_profit_pct)),
            ("Max drawdown", pct(r.max_drawdown_pct), t.text),
            ("Trades", r.total_trades.to_string(), t.text),
            ("Sharpe", num(r.sharpe, 3), t.text),
            ("Buy & hold", pct(r.buy_hold_pct), tone(t, r.buy_hold_pct)),
        ];
        cards_row(ui, t, &cards);
        if !self.lab.port_errors.is_empty() {
            ui.label(
                egui::RichText::new(self.lab.port_errors.join(" · "))
                    .color(t.text3)
                    .size(11.5),
            )
            .on_hover_text(self.lab.port_errors.join("\n"));
        }
        ui.add_space(4.0);
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), self.chart_h()), Sense::hover());
        let xs: Vec<f64> = r.equity.iter().map(|e| e.0 as f64).collect();
        let ys: Vec<f64> = r.equity.iter().map(|e| e.1).collect();
        curve_chart(
            ui,
            t,
            rect,
            &xs,
            &[(&ys, t.accent)],
            None,
            r.initial_capital,
            resp.hover_pos(),
            &|x| time_text(x as i64, &f.tz),
        );
        ui.add_space(6.0);
        let heads: Vec<String> = [
            "Symbol",
            "Net profit",
            "Return",
            "Trades",
            "Profitable",
            "Max drawdown",
            "Buy & hold",
        ]
        .map(String::from)
        .to_vec();
        let rows: Vec<Vec<(String, Color32)>> = r
            .legs
            .iter()
            .map(|l| {
                let s = &l.summary;
                vec![
                    (l.symbol.clone(), t.text),
                    (f.money(s.net_profit), tone(t, s.net_profit)),
                    (pct(s.net_profit_pct), tone(t, s.net_profit_pct)),
                    (s.total_trades.to_string(), t.text),
                    (pct(s.percent_profitable), t.text),
                    (pct(s.max_drawdown_pct), t.text),
                    (pct(l.buy_hold_pct), tone(t, l.buy_hold_pct)),
                ]
            })
            .collect();
        grid_table(ui, t, "lab-port", &heads, &rows);
    }
}

fn date(ms: i64, tz: &jiff::tz::TimeZone) -> String {
    jiff::Timestamp::from_millisecond(ms)
        .map(|ts| ts.to_zoned(tz.clone()).strftime("%Y-%m-%d").to_string())
        .unwrap_or_default()
}

/// The headless runner for the optimizer: the chart's strategy with some
/// inputs replaced.
fn runner(target: &Target, params: &[Param]) -> impl Fn(&[PValue], &[Bar]) -> Option<Broker> + Sync + use<> {
    let (meta, base, factory, tf, tz, forming_at) = (
        target.meta,
        target.inputs.clone(),
        target.factory.clone(),
        target.tf,
        target.tz.clone(),
        target.forming_at,
    );
    let params = params.to_vec();
    move |values: &[PValue], bars: &[Bar]| {
        let ins = inputs_with(meta, &base, &params, values);
        run_study(meta, &ins, &factory, bars, tf, &tz, forming_at)
    }
}

fn row(ui: &mut Ui, t: &Theme, label: &str, control: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(vec2(118.0, 24.0), Sense::hover());
        ui.painter().text(
            r.left_center(),
            Align2::LEFT_CENTER,
            label,
            theme::regular(13.0),
            t.text2,
        );
        control(ui);
    });
}

/// Low → high: from a faint wash to the accent.
fn heat_color(t: &Theme, k: f32) -> Color32 {
    let k = k.clamp(0.0, 1.0);
    let base = t.card;
    let lerp = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * (0.12 + 0.88 * k)) as u8;
    Color32::from_rgb(
        lerp(base.r(), t.accent.r()),
        lerp(base.g(), t.accent.g()),
        lerp(base.b(), t.accent.b()),
    )
}

fn cards_row(ui: &mut Ui, t: &Theme, cards: &[(&str, String, Color32)]) {
    let gap = 8.0;
    let avail = ui.available_width();
    let per_row = (((avail + gap) / (150.0 + gap)).floor() as usize).clamp(1, cards.len());
    let w = (avail - gap * (per_row - 1) as f32) / per_row as f32;
    for chunk in cards.chunks(per_row) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for (label, value, c) in chunk {
                kpi(ui, t, w, false, label, value, "", *c);
            }
        });
        ui.add_space(gap * 0.6);
    }
}

/// A simple table with a header, rows scrolling under it.
fn grid_table(ui: &mut Ui, t: &Theme, id: &str, heads: &[String], rows: &[Vec<(String, Color32)>]) {
    let w = ui.available_width();
    let first = (w * 0.26).clamp(120.0, 230.0);
    let col_w = ((w - first) / (heads.len().max(2) - 1) as f32).clamp(70.0, 160.0);
    let table_w = first + col_w * (heads.len() - 1) as f32;
    let row_h = 24.0;
    let x_of = |k: usize, r: Rect| {
        if k == 0 {
            (r.left() + 6.0, Align2::LEFT_CENTER)
        } else {
            (r.left() + first + col_w * k as f32 - 8.0, Align2::RIGHT_CENTER)
        }
    };
    egui::ScrollArea::horizontal().id_salt((id, "h")).show(ui, |ui| {
        ui.set_min_width(table_w);
        let (r, _) = ui.allocate_exact_size(vec2(table_w, row_h), Sense::hover());
        for (k, h) in heads.iter().enumerate() {
            let (x, a) = x_of(k, r);
            ui.painter()
                .text(pos2(x, r.center().y), a, h, theme::medium(12.0), t.text3);
        }
        ui.painter().hline(r.x_range(), r.bottom(), Stroke::new(1.0, t.border));
        {
            {
                for row in rows {
                    let (r, resp) = ui.allocate_exact_size(vec2(table_w, row_h), Sense::hover());
                    if resp.hovered() {
                        ui.painter().rect_filled(r, CornerRadius::same(4), t.hover_wash());
                    }
                    for (k, (text, c)) in row.iter().enumerate() {
                        let (x, a) = x_of(k, r);
                        ui.painter()
                            .text(pos2(x, r.center().y), a, text, theme::regular(12.5), *c);
                    }
                }
            }
        }
    });
}

/// Lines (and an optional shaded band) over `xs`, with a dashed baseline
/// and a hover read-out.
#[allow(clippy::too_many_arguments)]
fn curve_chart(
    ui: &Ui,
    t: &Theme,
    rect: Rect,
    xs: &[f64],
    lines: &[(&[f64], Color32)],
    band: Option<(&[f64], &[f64])>,
    base: f64,
    hover: Option<egui::Pos2>,
    x_label: &dyn Fn(f64) -> String,
) {
    let p = ui.painter_at(rect);
    p.rect(
        rect,
        CornerRadius::same(8),
        t.card,
        Stroke::new(1.0, t.border),
        egui::StrokeKind::Inside,
    );
    let n = xs.len();
    if n < 2 {
        return;
    }
    let plot = Rect::from_min_max(rect.min + vec2(10.0, 10.0), rect.max - vec2(84.0, 22.0));
    let mut lo = base;
    let mut hi = base;
    for v in lines
        .iter()
        .flat_map(|(ys, _)| ys.iter())
        .chain(band.iter().flat_map(|(a, b)| a.iter().chain(b.iter())))
    {
        if v.is_finite() {
            lo = lo.min(*v);
            hi = hi.max(*v);
        }
    }
    if hi <= lo {
        hi = lo + 1.0;
    }
    let pad = (hi - lo) * 0.06;
    let (lo, hi) = (lo - pad, hi + pad);
    let (x0, x1) = (xs[0], xs[n - 1]);
    let sx = |x: f64| plot.left() + ((x - x0) / (x1 - x0).max(1e-9)) as f32 * plot.width();
    let sy = |y: f64| plot.bottom() - ((y - lo) / (hi - lo)) as f32 * plot.height();
    // Grid and price labels.
    for k in 0..=4 {
        let v = lo + (hi - lo) * k as f64 / 4.0;
        let y = sy(v);
        p.hline(plot.x_range(), y, Stroke::new(1.0, t.border.gamma_multiply(0.5)));
        p.text(
            pos2(plot.right() + 8.0, y),
            Align2::LEFT_CENTER,
            crate::strategy::grouped(v, 0),
            theme::regular(11.0),
            t.text3,
        );
    }
    let by = sy(base);
    let mut x = plot.left();
    while x < plot.right() {
        p.hline(x..=(x + 4.0).min(plot.right()), by, Stroke::new(1.0, t.text3));
        x += 8.0;
    }
    // At most ~2 points per pixel.
    let step = (n / (plot.width() as usize * 2).max(1)).max(1);
    if let Some((a, b)) = band {
        let mut mesh = egui::Mesh::default();
        let fill = t.accent.gamma_multiply(0.15);
        let idx: Vec<usize> = (0..n).step_by(step).chain(std::iter::once(n - 1)).collect();
        for (k, &i) in idx.iter().enumerate() {
            mesh.colored_vertex(pos2(sx(xs[i]), sy(a[i])), fill);
            mesh.colored_vertex(pos2(sx(xs[i]), sy(b[i])), fill);
            if k > 0 {
                let j = (k as u32) * 2;
                mesh.add_triangle(j - 2, j - 1, j);
                mesh.add_triangle(j - 1, j, j + 1);
            }
        }
        p.add(mesh);
    }
    for (ys, c) in lines {
        let pts: Vec<egui::Pos2> = (0..n)
            .step_by(step)
            .chain(std::iter::once(n - 1))
            .filter(|i| ys[*i].is_finite())
            .map(|i| pos2(sx(xs[i]), sy(ys[i])))
            .collect();
        p.add(egui::Shape::line(pts, Stroke::new(1.6, *c)));
    }
    if let Some(h) = hover.filter(|h| plot.contains(*h)) {
        let x = x0 + ((h.x - plot.left()) / plot.width()) as f64 * (x1 - x0);
        let i = xs.partition_point(|v| *v < x).min(n - 1);
        let px = sx(xs[i]);
        p.vline(px, plot.y_range(), Stroke::new(1.0, t.text3));
        let mut text = x_label(xs[i]);
        for (ys, _) in lines {
            text.push_str(&format!("\n{}", crate::strategy::grouped(ys[i], 2)));
        }
        if let Some((a, b)) = band {
            text.push_str(&format!(
                "\n5–95%: {} – {}",
                crate::strategy::grouped(a[i], 0),
                crate::strategy::grouped(b[i], 0)
            ));
        }
        let g = p.layout_no_wrap(text, theme::regular(11.5), t.text);
        let size = g.size() + vec2(14.0, 10.0);
        let mut at = pos2(px + 10.0, plot.top() + 6.0);
        if at.x + size.x > rect.right() {
            at.x = px - 10.0 - size.x;
        }
        let r = Rect::from_min_size(at, size);
        p.rect(
            r,
            CornerRadius::same(6),
            t.elevated,
            Stroke::new(1.0, t.border),
            egui::StrokeKind::Inside,
        );
        p.galley(r.min + vec2(7.0, 5.0), g, t.text);
    }
    p.text(
        pos2(plot.left(), rect.bottom() - 6.0),
        Align2::LEFT_BOTTOM,
        x_label(x0),
        theme::regular(11.0),
        t.text3,
    );
    p.text(
        pos2(plot.right(), rect.bottom() - 6.0),
        Align2::RIGHT_BOTTOM,
        x_label(x1),
        theme::regular(11.0),
        t.text3,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_ranges_bracket_the_default() {
        let spec = opentern_series::study::InputSpec {
            id: "len",
            title: "Length",
            kind: InputKind::Int { min: 1, max: 500 },
            default: Value::Int(14),
            group: None,
        };
        let r = default_row(&spec).expect("row");
        assert_eq!(
            r.range,
            Range::Int {
                from: 7,
                to: 28,
                step: 2
            }
        );
        let f = opentern_series::study::InputSpec {
            id: "mult",
            title: "Mult",
            kind: InputKind::Float {
                min: 0.0,
                max: 50.0,
                step: 0.1,
            },
            default: Value::Float(2.0),
            group: None,
        };
        match default_row(&f).expect("row").range {
            Range::Float { from, to, step } => {
                assert_eq!((from, to), (1.0, 4.0));
                assert!((step - 0.3).abs() < 1e-9, "{step}");
            }
            r => panic!("{r:?}"),
        }
    }

    /// The optimizer reruns a real Pine strategy headlessly, and the best
    /// run's inputs reproduce its numbers on the chart.
    #[test]
    fn optimizes_a_pine_strategy() {
        let src = r#"//@version=5
strategy("Cross", overlay=true, initial_capital=10000)
fast = input.int(5, "Fast", minval=1)
slow = input.int(20, "Slow", minval=1)
if ta.crossover(ta.sma(close, fast), ta.sma(close, slow))
    strategy.entry("L", strategy.long)
if ta.crossunder(ta.sma(close, fast), ta.sma(close, slow))
    strategy.close("L")
"#;
        let script = opentern_pine::compile(src, "cross.pine", opentern_pine::Env::default()).expect("compiles");
        let meta = script.meta();
        let bars: Vec<Bar> = (0..800)
            .map(|i| {
                let x = i as f64;
                let c = 100.0 + x * 0.03 + (x / 9.0).sin() * 5.0 + (x / 31.0).cos() * 7.0;
                Bar {
                    time: 1_600_000_000_000 + i as i64 * 3_600_000,
                    open: c - 0.2,
                    high: c + 1.0,
                    low: c - 1.0,
                    close: c,
                    volume: 10.0,
                }
            })
            .collect();
        let target = Target {
            uid: 1,
            meta,
            inputs: meta.default_inputs(),
            factory: script.factory(),
            build: None,
            tf: Timeframe::parse("1h").expect("tf"),
            tz: jiff::tz::TimeZone::UTC,
            forming_at: None,
        };
        let setup = Setup {
            params: vec![
                Param {
                    id: meta.inputs[0].id.to_string(),
                    title: "Fast".into(),
                    range: Range::Int {
                        from: 3,
                        to: 12,
                        step: 3,
                    },
                },
                Param {
                    id: meta.inputs[1].id.to_string(),
                    title: "Slow".into(),
                    range: Range::Int {
                        from: 20,
                        to: 50,
                        step: 10,
                    },
                },
            ],
            method: Method::Grid,
            objective: Objective::NetProfit,
            min_trades: 1,
            seed: 1,
        };
        let run = runner(&target, &setup.params);
        let out = optimize::optimize(&setup, &bars, &run, &Progress::default()).expect("runs");
        assert_eq!(out.trials.len(), 16);
        assert!(out.trials.iter().all(|t| !t.failed));
        let best = out.best().expect("best");
        let ins = inputs_with(meta, &meta.default_inputs(), &setup.params, &best.values);
        let b = run_study(meta, &ins, &script.factory(), &bars, target.tf, &target.tz, None).expect("runs");
        let s = opentern_engine::quant::Summary::of_run(&b, &bars);
        assert_eq!(s.net_profit, best.summary.net_profit);
        // Different inputs give different results.
        let distinct: std::collections::HashSet<String> = out
            .trials
            .iter()
            .map(|t| format!("{:.6}", t.summary.net_profit))
            .collect();
        assert!(distinct.len() > 4, "{distinct:?}");
    }

    /// The whole flow in the real app: Strategy settings override the
    /// script's properties, and every lab mode runs and shows its results.
    #[test]
    fn strategy_settings_and_every_lab_mode() {
        let mut d = crate::drive::Driver::new("lab");
        let dir = d.app.scripts_dir();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("cross.pine"),
            "//@version=5\nstrategy(\"Cross\", overlay = true, initial_capital = 20000)\n\
             fast = input.int(5, \"Fast\", minval = 1)\nslow = input.int(20, \"Slow\", minval = 1)\n\
             f = ta.sma(close, fast)\ns = ta.sma(close, slow)\n\
             if ta.crossover(f, s)\n    strategy.entry(\"L\", strategy.long)\nif ta.crossunder(f, s)\n    strategy.close(\"L\")\n",
        )
        .unwrap();
        d.app.add_script("cross.pine");
        d.wait(|a| a.strategies.runs.iter().any(|r| !r.broker.trades.is_empty()));
        assert_eq!(d.app.strategies.runs[0].broker.props.initial_capital, 20_000.0);

        // Strategy settings: the dialog opens; overrides rerun the script.
        let uid = d.app.strategies.runs[0].uid;
        d.app.open_strategy_props(uid);
        assert!(matches!(d.app.modal, Some(crate::dialogs::Modal::StrategyProps(_))));
        d.frame(vec![]);
        d.app.modal = None;
        d.app.set_strategy_overrides(
            "cross.pine",
            serde_json::json!({"initial_capital": 5000.0, "commission_value": 0.1}),
        );
        d.wait(|a| {
            a.strategies
                .runs
                .first()
                .is_some_and(|r| r.broker.props.initial_capital == 5000.0)
        });
        assert_eq!(d.app.strategies.runs[0].broker.props.commission_value, 0.1);
        assert_eq!(d.app.strategy_overrides("cross.pine")["initial_capital"], 5000.0);

        // Optimize, then put the best inputs on the chart.
        d.app.layout.tab = Some(crate::shell::Tab::Lab);
        d.app.lab_autorun("optimize");
        d.wait(|a| a.lab.opt.is_some());
        let best = d
            .app
            .lab
            .opt
            .as_ref()
            .and_then(|(_, o, _)| o.best().cloned())
            .expect("a best run");
        d.app.lab_apply(&best.values);
        d.wait(|a| {
            a.strategies
                .runs
                .first()
                .is_some_and(|r| (r.report.all.net_profit - best.summary.net_profit).abs() < 1e-6)
        });
        d.app.lab.view = OptView::Heatmap;
        for _ in 0..3 {
            d.frame(vec![]);
        }

        // The other modes.
        d.app.lab.setup.in_sample = 200;
        d.app.lab.setup.out_sample = 50;
        for (mode, done) in [
            (Mode::WalkForward, (|a: &App| a.lab.walk.is_some()) as fn(&App) -> bool),
            (Mode::MonteCarlo, |a: &App| a.lab.mc.is_some()),
            (Mode::Portfolio, |a: &App| a.lab.port.is_some()),
        ] {
            d.app.lab.setup.mode = mode;
            d.app.lab_run();
            d.wait(done);
            assert!(d.app.lab.job.is_none());
            for _ in 0..3 {
                d.frame(vec![]);
            }
        }
        let walk = &d.app.lab.walk.as_ref().unwrap().1;
        assert!(!walk.windows.is_empty(), "{}", d.app.lab.status);
        let port = d.app.lab.port.as_ref().unwrap();
        assert!(port.legs.len() >= 2, "{}", d.app.lab.status);
        // Each symbol got an equal share of the (overridden) capital.
        assert!((port.initial_capital - 5000.0).abs() < 1e-6);
        // Exports land in the workspace.
        d.app.lab_export(false);
        assert!(d.app.lab.status.starts_with("Saved to"), "{}", d.app.lab.status);
        assert_eq!(d.app.panics, 0);
    }
}

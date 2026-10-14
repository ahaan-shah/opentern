//! Pine scripts on the chart. A script compiles (opentern-pine) into a
//! study like any built-in: the chart steps it bar by bar and recomputes
//! the forming bar on every tick. It recompiles when the file is saved
//! here or in another editor, and when the symbol changes (`syminfo.*`).

use crate::app::App;
use crate::chart::StudyLook;
use opentern_core::AssetKind;
use opentern_pine::{Env, Kind, PineStudy};
use opentern_series::study::{Category, Format, Instance, Meta, Placement};
use std::time::{Duration, Instant};

fn kind_name(k: AssetKind) -> &'static str {
    match k {
        AssetKind::Crypto => "crypto",
        AssetKind::Stock => "stock",
        AssetKind::Fund => "fund",
        AssetKind::Index => "index",
        AssetKind::Forex => "forex",
        AssetKind::Futures => "futures",
    }
}

/// `syminfo.*` for `s`, without libraries.
pub(crate) fn env_of(s: &opentern_core::Symbol) -> Env {
    Env {
        tickerid: s.id.clone(),
        description: s.description.clone(),
        mintick: if s.tick_size > 0.0 { s.tick_size } else { 0.01 },
        currency: s.currency.clone(),
        kind: kind_name(s.kind).into(),
        timezone: s.timezone.clone(),
        libraries: Vec::new(),
    }
}

impl App {
    fn pine_env(&self) -> Env {
        self.pine_env_for(&self.chart.symbol)
    }

    /// What `syminfo.*` says when the script runs on `s`.
    pub(crate) fn pine_env_for(&self, s: &opentern_core::Symbol) -> Env {
        Env {
            libraries: self.pine_libraries(),
            ..env_of(s)
        }
    }

    /// Library scripts in the scripts folder, by their `library()` title.
    fn pine_libraries(&self) -> Vec<(String, String)> {
        let Ok(dir) = std::fs::read_dir(self.scripts_dir()) else {
            return Vec::new();
        };
        let mut out: Vec<(String, String)> = dir
            .flatten()
            .filter(|e| e.path().extension().is_some_and(|x| x == "pine"))
            .filter_map(|e| std::fs::read_to_string(e.path()).ok())
            .filter(|src| src.contains("library("))
            .filter_map(|src| opentern_pine::library_title(&src).map(|t| (t, src)))
            .collect();
        out.sort();
        out
    }

    fn env_key(env: &Env) -> String {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        env.libraries.hash(&mut h);
        format!(
            "{}|{}|{}|{}|{:x}",
            env.tickerid,
            env.mintick,
            env.currency,
            env.timezone,
            h.finish()
        )
    }

    /// Compiles script `i` and puts (or updates) its study on the chart.
    pub(crate) fn compile_pine(&mut self, i: usize) {
        let path = self.scripts_dir().join(&self.scripts[i].file);
        let env = self.pine_env();
        let run = &mut self.scripts[i];
        run.mtime = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
        run.env_key = Self::env_key(&env);
        run.checked = Some(Instant::now());
        let src = match std::fs::read_to_string(&path) {
            Ok(s) => s,
            Err(e) => {
                run.compile_error = Some(format!("Couldn't read {}: {e}", run.file));
                run.error = run.compile_error.clone();
                return;
            }
        };
        let script = match opentern_pine::compile(&src, &run.file, env) {
            Ok(s) => s,
            Err(e) => {
                // Keep the last good version on the chart; show the error.
                run.compile_error = Some(e.to_string());
                run.error = run.compile_error.clone();
                if run.uid.is_none() {
                    self.placeholder(i);
                }
                return;
            }
        };
        run.compile_error = None;
        run.error = None;
        run.kind = match script.kind() {
            Kind::Strategy => "strategy",
            Kind::Indicator => "indicator",
            Kind::Library => "library",
        }
        .into();
        run.props = script.strategy_props().cloned().unwrap_or_default();
        let script = if script.kind() == Kind::Strategy {
            let file = run.file.clone();
            script.with_props(&self.strategy_overrides(&file))
        } else {
            script
        };
        let run = &mut self.scripts[i];
        if script.kind() == Kind::Library {
            run.compile_error = Some("This is a library: import it from another script.".into());
            run.error = run.compile_error.clone();
            return;
        }
        let meta = script.meta();
        let restore = run.restore.take();
        let inputs = match (&restore, run.uid) {
            (Some(r), _) => r.inputs.clone(),
            (None, Some(uid)) => self
                .chart
                .studies
                .iter()
                .find(|s| s.uid == uid)
                .map(|s| s.inst.inputs.clone())
                .unwrap_or_default(),
            (None, None) => meta.default_inputs(),
        };
        let inst = Instance::with_factory(meta, &inputs, script.factory());
        let run = &mut self.scripts[i];
        run.pine = Some(script);
        match run.uid {
            Some(uid) => {
                let placement = self
                    .chart
                    .studies
                    .iter()
                    .find(|s| s.uid == uid)
                    .map(|s| s.meta().placement);
                if placement == Some(meta.placement) {
                    self.chart.replace_instance(uid, inst);
                } else {
                    // Overlay ↔ pane needs a new place.
                    self.chart.remove_study(uid);
                    let (look, pane, visible) = match &restore {
                        Some(r) => (r.look.clone(), Some(r.pane), r.visible),
                        None => (StudyLook::defaults(meta), None, true),
                    };
                    self.scripts[i].uid = Some(self.chart.add_instance(inst, look, pane, visible));
                }
            }
            None => {
                let (look, pane, visible) = match &restore {
                    Some(r) => (r.look.clone(), Some(r.pane), r.visible),
                    None => (StudyLook::defaults(meta), None, true),
                };
                run.uid = Some(self.chart.add_instance(inst, look, pane, visible));
            }
        }
        self.after_pine_run(i);
    }

    /// A script that has never compiled still gets a legend row (named
    /// after its file) so its error shows on the chart; the real study
    /// replaces it once it compiles.
    fn placeholder(&mut self, i: usize) {
        let run = &self.scripts[i];
        let name = run.file.trim_end_matches(".pine").to_string();
        let meta: &'static Meta = Box::leak(Box::new(Meta {
            id: Box::leak(format!("pine:{}", run.file).into_boxed_str()),
            name: Box::leak(name.clone().into_boxed_str()),
            short: Box::leak(name.into_boxed_str()),
            category: Category::Oscillators,
            placement: Placement::Overlay,
            format: Format::Price,
            inputs: vec![],
            plots: vec![],
            hlines: vec![],
            fills: vec![],
            build: |_| Box::new(opentern_series::study::NullStudy),
        }));
        let (pane, visible) = match &run.restore {
            Some(r) => (Some(r.pane), r.visible),
            None => (None, true),
        };
        let inst = Instance::external(meta, &Default::default());
        let uid = self.chart.add_instance(inst, StudyLook::defaults(meta), pane, visible);
        self.scripts[i].uid = Some(uid);
    }

    /// Every frame: recompile on outside edits (checked once a second) or a
    /// new symbol, and pick up runtime errors and logs.
    pub(crate) fn poll_pine(&mut self, i: usize) {
        let due = self.scripts[i]
            .checked
            .is_none_or(|t| t.elapsed() > Duration::from_secs(1));
        if due {
            self.scripts[i].checked = Some(Instant::now());
            let path = self.scripts_dir().join(&self.scripts[i].file);
            let mtime = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
            let env_key = Self::env_key(&self.pine_env());
            let run = &self.scripts[i];
            if mtime != run.mtime || env_key != run.env_key {
                self.compile_pine(i);
                return;
            }
        }
        self.after_pine_run(i);
    }

    /// Runtime errors and `log.*` output from the study's latest state.
    fn after_pine_run(&mut self, i: usize) {
        let Some(uid) = self.scripts[i].uid else { return };
        let Some(st) = self
            .chart
            .studies
            .iter()
            .find(|s| s.uid == uid)
            .and_then(|s| s.inst.latest_state())
            .and_then(|a| a.downcast_ref::<PineStudy>())
        else {
            return;
        };
        let error = st.error().map(|e| e.to_string());
        let logs = st.logs().join("\n");
        let run = &mut self.scripts[i];
        // A compile error wins (the last good version is what's running).
        run.error = run.compile_error.clone().or(error);
        if !logs.is_empty() {
            run.log = logs;
        }
    }
}

//! Scripts on the chart. A script file (Python now, Pine next) becomes a
//! study: Python runs in a worker process and its plots are written into an
//! "external" study; the study's declaration (inputs, plots, levels) comes
//! from the script and is rebuilt when the script changes.

use crate::app::App;
use crate::chart::{SavedStudy, StudyLook};
use opentern_script::python;
use opentern_series::Rgba;
use opentern_series::study::{
    Category, Format, HLineSpec, InputKind, InputSpec, Inputs, Instance, LineStyle, Meta, Offset, Output, Placement,
    PlotSpec, PlotStyle, Value,
};
use std::path::PathBuf;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lang {
    Python,
    Pine,
}

impl Lang {
    pub fn of(file: &str) -> Option<Lang> {
        if file.ends_with(".py") {
            Some(Lang::Python)
        } else if file.ends_with(".pine") {
            Some(Lang::Pine)
        } else {
            None
        }
    }
}

/// One script on the chart.
pub struct ScriptRun {
    pub lang: Lang,
    pub file: String,
    runner: Option<python::Runner>,
    /// The chart study, once the script has produced its first output.
    pub uid: Option<u64>,
    /// How it was saved (inputs, look, pane), to restore on first output.
    pub(crate) restore: Option<SavedStudy>,
    desc: Option<python::Meta>,
    signature: String,
    pub error: Option<String>,
    pub log: String,
    sent_version: u64,
    sent_inputs: Option<Inputs>,
    last_run: Option<Instant>,
    in_flight: bool,
    /// The last output (for strategies: the orders).
    pub output: Option<python::Output>,
    pub kind: String,
    pub props: serde_json::Value,
    /// Pine: the compiled script, the file's modification time and the
    /// symbol it was compiled for (`syminfo.*` is baked in).
    pub(crate) pine: Option<opentern_pine::Script>,
    /// Pine: the last compile error (the last good version keeps running).
    pub(crate) compile_error: Option<String>,
    pub(crate) mtime: Option<std::time::SystemTime>,
    pub(crate) env_key: String,
    pub(crate) checked: Option<Instant>,
    /// Bumped on every Python result (strategy runs rebuild on it).
    pub(crate) out_seq: u64,
}

fn leak(s: impl Into<String>) -> &'static str {
    Box::leak(s.into().into_boxed_str())
}

pub fn parse_color(s: &str) -> Rgba {
    let h = s.trim().trim_start_matches('#');
    match h.len() {
        6 => u32::from_str_radix(h, 16).map(Rgba::hex).unwrap_or(Rgba::hex(0x2962FF)),
        8 => u32::from_str_radix(h, 16).map(Rgba).unwrap_or(Rgba::hex(0x2962FF)),
        _ => match s {
            "red" => Rgba::hex(0xF23645),
            "green" => Rgba::hex(0x089981),
            "orange" => Rgba::hex(0xFF9800),
            "purple" => Rgba::hex(0x9C27B0),
            "yellow" => Rgba::hex(0xFFEB3B),
            "gray" | "grey" => Rgba::hex(0x787B86),
            "white" => Rgba::hex(0xFFFFFF),
            "black" => Rgba::hex(0x363A45),
            _ => Rgba::hex(0x2962FF),
        },
    }
}

fn plot_style(s: &str) -> PlotStyle {
    match s {
        "linebr" => PlotStyle::LineBr,
        "stepline" | "step" => PlotStyle::StepLine,
        "histogram" | "hist" => PlotStyle::Histogram,
        "columns" => PlotStyle::Columns,
        "area" => PlotStyle::Area,
        "circles" => PlotStyle::Circles,
        "cross" => PlotStyle::Cross,
        _ => PlotStyle::Line,
    }
}

fn line_style(s: &str) -> LineStyle {
    match s {
        "solid" => LineStyle::Solid,
        "dotted" => LineStyle::Dotted,
        _ => LineStyle::Dashed,
    }
}

fn input_spec(d: &python::InputDesc) -> InputSpec {
    let num = |v: &serde_json::Value| v.as_f64().unwrap_or(0.0);
    let (kind, default) = match d.kind.as_str() {
        "int" => (
            InputKind::Int {
                min: d.min.map(|v| v as i64).unwrap_or(-1_000_000_000),
                max: d.max.map(|v| v as i64).unwrap_or(1_000_000_000),
            },
            Value::Int(num(&d.default) as i64),
        ),
        "float" => (
            InputKind::Float {
                min: d.min.unwrap_or(-1e12),
                max: d.max.unwrap_or(1e12),
                step: d.step.unwrap_or(0.1),
            },
            Value::Float(num(&d.default)),
        ),
        "bool" => (InputKind::Bool, Value::Bool(d.default.as_bool().unwrap_or(false))),
        "source" => (
            InputKind::Source,
            Value::Text(d.default.as_str().unwrap_or("close").into()),
        ),
        _ if !d.options.is_empty() => {
            let opts: Vec<&'static str> = d.options.iter().map(|o| leak(o.clone())).collect();
            (
                InputKind::Choice(Box::leak(opts.into_boxed_slice())),
                Value::Text(d.default.as_str().unwrap_or("").into()),
            )
        }
        _ => (InputKind::Text, Value::Text(d.default.as_str().unwrap_or("").into())),
    };
    InputSpec {
        id: leak(d.id.clone()),
        title: leak(d.title.clone()),
        kind,
        default,
        group: None,
    }
}

/// A study declaration for a Python script's latest description and output.
fn python_meta(file: &str, desc: &python::Meta, out: &python::Output) -> &'static Meta {
    let plots = out
        .plots
        .iter()
        .map(|p| PlotSpec {
            title: leak(p.title.clone()),
            style: plot_style(&p.style),
            colors: vec![("Color", parse_color(&p.color))],
            width: p.width.max(1),
            offset: Offset::Fixed(0),
            visible: true,
            base: 0.0,
            shape: None,
        })
        .collect();
    let hlines = out
        .hlines
        .iter()
        .map(|h| HLineSpec {
            title: leak(h.title.clone()),
            value: h.value,
            color: parse_color(&h.color),
            style: line_style(&h.style),
            width: 1,
            visible: true,
        })
        .collect();
    let short = if desc.short.is_empty() {
        desc.name.clone()
    } else {
        desc.short.clone()
    };
    Box::leak(Box::new(Meta {
        id: leak(format!("py:{file}")),
        name: leak(desc.name.clone()),
        short: leak(short),
        category: Category::Oscillators,
        placement: if desc.overlay {
            Placement::Overlay
        } else {
            Placement::Pane
        },
        format: desc.precision.map(Format::Fixed).unwrap_or(Format::Price),
        inputs: desc.inputs.iter().map(input_spec).collect(),
        plots,
        hlines,
        fills: vec![],
        build: |_| Box::new(opentern_series::study::NullStudy),
    }))
}

fn signature(desc: &python::Meta, out: &python::Output) -> String {
    let mut s = format!("{}|{}|{:?}|", desc.name, desc.overlay, desc.precision);
    for i in &desc.inputs {
        s += &format!("{}:{}:{}|", i.id, i.kind, i.options.join(","));
    }
    for p in &out.plots {
        s += &format!("{}:{}:{}:{}|", p.title, p.color, p.width, p.style);
    }
    for h in &out.hlines {
        s += &format!("{}:{}:{}|", h.title, h.value, h.color);
    }
    s
}

fn inputs_json(i: &Inputs) -> serde_json::Value {
    let mut m = serde_json::Map::new();
    for (k, v) in &i.values {
        let j = match v {
            Value::Int(x) => serde_json::json!(x),
            Value::Float(x) => serde_json::json!(x),
            Value::Bool(x) => serde_json::json!(x),
            Value::Text(x) => serde_json::json!(x),
        };
        m.insert(k.clone(), j);
    }
    serde_json::Value::Object(m)
}

impl App {
    pub fn scripts_dir(&self) -> PathBuf {
        self.store.dir().join("scripts")
    }

    fn sdk_dir(&self) -> PathBuf {
        self.store.dir().join("python")
    }

    /// Script files in the workspace, sorted by name.
    pub fn script_files(&self) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(self.scripts_dir())
            .map(|d| {
                d.flatten()
                    .filter_map(|e| e.file_name().into_string().ok())
                    .filter(|n| Lang::of(n).is_some())
                    .collect()
            })
            .unwrap_or_default();
        v.sort_by_key(|n| n.to_lowercase());
        v
    }

    /// Puts a script on the chart.
    pub fn add_script(&mut self, file: &str) {
        self.start_script(file, None);
    }

    fn start_script(&mut self, file: &str, restore: Option<SavedStudy>) {
        let Some(lang) = Lang::of(file) else { return };
        let path = self.scripts_dir().join(file);
        let runner = match lang {
            Lang::Python => {
                let ctx = self.egui_ctx.clone();
                Some(python::Runner::start(
                    path,
                    self.sdk_dir(),
                    std::sync::Arc::new(move || ctx.request_repaint()),
                ))
            }
            Lang::Pine => None,
        };
        self.scripts.push(ScriptRun {
            lang,
            file: file.to_string(),
            runner,
            uid: None,
            restore,
            desc: None,
            signature: String::new(),
            error: None,
            log: String::new(),
            sent_version: u64::MAX,
            sent_inputs: None,
            last_run: None,
            in_flight: false,
            output: None,
            kind: String::new(),
            props: serde_json::Value::Null,
            pine: None,
            compile_error: None,
            mtime: None,
            env_key: String::new(),
            checked: None,
            out_seq: 0,
        });
        if lang == Lang::Pine {
            let i = self.scripts.len() - 1;
            self.compile_pine(i);
        }
    }

    /// Starts the scripts of a freshly loaded layout, and forgets scripts
    /// whose study was removed from the chart.
    pub(crate) fn sync_scripts(&mut self) {
        let pending = std::mem::take(&mut self.chart.pending_scripts);
        for s in pending {
            let file = s.id.split_once(':').map(|(_, f)| f.to_string()).unwrap_or_default();
            self.start_script(&file, Some(s));
        }
        let uids: Vec<u64> = self.chart.studies.iter().map(|s| s.uid).collect();
        self.scripts.retain(|r| r.uid.is_none_or(|u| uids.contains(&u)));
    }

    /// Reads script results and starts new runs when the data or the
    /// inputs changed. Called every frame.
    pub(crate) fn poll_scripts(&mut self) {
        self.sync_scripts();
        self.publish_script_errors();
        let n = self.chart.bars.len();
        for i in 0..self.scripts.len() {
            if self.scripts[i].lang == Lang::Pine {
                self.poll_pine(i);
                continue;
            }
            let events = self.scripts[i].runner.as_ref().map(|r| r.poll()).unwrap_or_default();
            for e in events {
                let run = &mut self.scripts[i];
                match e {
                    python::Event::Described(d) => {
                        run.error = None;
                        run.kind = d.kind.clone();
                        run.props = d.props.clone();
                        run.desc = Some(d);
                        // A reload: run again even if nothing else changed.
                        run.sent_version = u64::MAX;
                    }
                    python::Event::Output { meta, out, .. } => {
                        run.in_flight = false;
                        run.error = None;
                        run.kind = meta.kind.clone();
                        run.props = meta.props.clone();
                        let sig = signature(&meta, &out);
                        let file = run.file.clone();
                        let restore = run.restore.take();
                        let rebuild = run.uid.is_none() || sig != run.signature;
                        run.signature = sig;
                        run.desc = Some(meta.clone());
                        if rebuild {
                            let m = python_meta(&file, &meta, &out);
                            let inputs = match (&restore, run.uid) {
                                (Some(r), _) => r.inputs.clone(),
                                (None, Some(uid)) => self
                                    .chart
                                    .studies
                                    .iter()
                                    .find(|s| s.uid == uid)
                                    .map(|s| s.inst.inputs.clone())
                                    .unwrap_or_default(),
                                (None, None) => m.default_inputs(),
                            };
                            let inst = Instance::external(m, &inputs);
                            let run = &mut self.scripts[i];
                            match run.uid {
                                Some(uid) => {
                                    // Overlay ↔ pane changes need a new place.
                                    let placement = self
                                        .chart
                                        .studies
                                        .iter()
                                        .find(|s| s.uid == uid)
                                        .map(|s| s.meta().placement);
                                    if placement == Some(m.placement) {
                                        self.chart.replace_instance(uid, inst);
                                    } else {
                                        self.chart.remove_study(uid);
                                        let look = StudyLook::defaults(m);
                                        self.scripts[i].uid = Some(self.chart.add_instance(inst, look, None, true));
                                    }
                                }
                                None => {
                                    let (look, pane, visible) = match &restore {
                                        Some(r) => (r.look.clone(), Some(r.pane), r.visible),
                                        None => (StudyLook::defaults(m), None, true),
                                    };
                                    run.uid = Some(self.chart.add_instance(inst, look, pane, visible));
                                }
                            }
                        }
                        let run = &mut self.scripts[i];
                        let output = Output {
                            plots: out
                                .plots
                                .iter()
                                .map(|p| p.values.iter().map(|v| v.unwrap_or(f64::NAN)).collect())
                                .collect(),
                            colors: out.plots.iter().map(|p| vec![0; p.values.len()]).collect(),
                            ..Default::default()
                        };
                        run.output = Some(out);
                        run.out_seq += 1;
                        if let Some(uid) = run.uid
                            && let Some(s) = self.chart.study_mut(uid)
                        {
                            s.inst.set_output(output, n);
                        }
                    }
                    python::Event::Error(m) => {
                        run.in_flight = false;
                        run.error = Some(m);
                    }
                    python::Event::Log(l) => {
                        run.log.push_str(&l);
                        if run.log.len() > 8000 {
                            run.log.drain(..run.log.len() - 8000);
                        }
                    }
                }
            }
            // A new run when the data or the inputs changed: right away for
            // new history or settings, at most twice a second for ticks.
            let inputs = self.scripts[i]
                .uid
                .and_then(|u| self.chart.studies.iter().find(|s| s.uid == u))
                .map(|s| s.inst.inputs.clone())
                .or_else(|| self.scripts[i].restore.as_ref().map(|r| r.inputs.clone()))
                .unwrap_or_default();
            let run = &mut self.scripts[i];
            let data_changed = run.sent_version != self.chart.data_version;
            let inputs_changed = run.sent_inputs.as_ref() != Some(&inputs);
            let soon = run.last_run.is_none_or(|t| t.elapsed() > Duration::from_millis(500));
            if n > 0 && run.desc.is_some() && (inputs_changed || (data_changed && soon && !run.in_flight)) {
                if let Some(r) = run.runner.as_mut() {
                    r.run(self.chart.bars_arc(), inputs_json(&inputs));
                }
                run.sent_version = self.chart.data_version;
                run.sent_inputs = Some(inputs);
                run.last_run = Some(Instant::now());
                run.in_flight = true;
            } else if data_changed && !soon {
                self.egui_ctx.request_repaint_after(Duration::from_millis(500));
            }
        }
    }

    /// Script errors, by chart study, for the legend.
    pub(crate) fn publish_script_errors(&mut self) {
        self.chart.study_errors = self
            .scripts
            .iter()
            .filter_map(|r| Some((r.uid?, r.error.clone()?)))
            .collect();
    }
}

/// Builds a non-Pine in-process strategy for another symbol (portfolio
/// runs). Python runs out of process, so it has none.
pub(crate) fn other_builder(
    lang: Lang,
    _src: String,
    _file: String,
    _over: serde_json::Value,
) -> Option<crate::lab::Build> {
    match lang {
        Lang::Python | Lang::Pine => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colours_and_styles() {
        assert_eq!(parse_color("#2962FF"), Rgba::hex(0x2962FF));
        assert_eq!(parse_color("#11223344"), Rgba(0x11223344));
        assert_eq!(parse_color("red"), Rgba::hex(0xF23645));
        assert_eq!(plot_style("histogram"), PlotStyle::Histogram);
        assert_eq!(line_style("solid"), LineStyle::Solid);
    }
}

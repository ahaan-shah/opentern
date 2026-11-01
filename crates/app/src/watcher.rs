//! Watches alerts with no chart: a data session per (symbol, interval) an
//! alert needs, and each plot or script alert's study run on those bars.
//! The app uses it for alerts whose symbol or study isn't on the chart;
//! the background process (`opentern --daemon`) for every alert. Nothing
//! here touches the UI.

use opentern_core::{Bar, Symbol, Timeframe};
use opentern_data::{Event, Request};
use opentern_engine::alerts::{Alert, Condition};
use opentern_pine::PineStudy;
use opentern_series::study::{Inputs, Instance, sessions};
use std::collections::HashMap;
use std::path::PathBuf;

/// What an alert sees now.
pub enum Probe {
    /// (value, level, bar time) for price, line and plot alerts.
    Values(f64, f64, i64),
    /// Script alerts: every (bar time, message, alertcondition title) the
    /// script raised, and the forming bar's time.
    Script(Vec<(i64, String, String)>, i64),
    /// Not known yet (no bars, or the study failed).
    Nothing,
}

struct Feed {
    session: u64,
    symbol: Symbol,
    tf: Timeframe,
    bars: Vec<Bar>,
    /// Bumped on every change; `resets` when the whole series is replaced.
    version: u64,
    resets: u64,
}

struct Run {
    /// The spec and feed it was built for (rebuilt if either changes).
    key: String,
    inst: Option<Instance>,
    version: u64,
    resets: u64,
    error: Option<String>,
}

pub struct Watcher {
    feeds: HashMap<String, Feed>,
    runs: HashMap<i64, Run>,
    scripts: PathBuf,
}

/// The data an alert needs: its symbol on its interval.
fn feed_key(a: &Alert) -> String {
    format!("{}|{}", a.symbol, a.tf)
}

impl Watcher {
    pub fn new(scripts: PathBuf) -> Watcher {
        Watcher {
            feeds: HashMap::new(),
            runs: HashMap::new(),
            scripts,
        }
    }

    pub fn set_scripts_dir(&mut self, dir: PathBuf) {
        if dir != self.scripts {
            self.scripts = dir;
            self.runs.clear();
        }
    }

    /// Opens sessions for these alerts' symbols and closes the rest.
    pub fn sync<'a>(&mut self, alerts: impl Iterator<Item = &'a Alert>, send: &dyn Fn(Request)) {
        let mut wanted: HashMap<String, (Symbol, Timeframe)> = HashMap::new();
        let mut ids = Vec::new();
        for a in alerts {
            ids.push(a.id);
            let Some(tf) = Timeframe::parse(&a.tf) else { continue };
            let sym = a.sym.clone().unwrap_or_else(|| Symbol::bare(&a.symbol));
            wanted.entry(feed_key(a)).or_insert((sym, tf));
        }
        self.feeds.retain(|k, f| {
            let keep = wanted.contains_key(k);
            if !keep {
                send(Request::Close { chart: f.session });
            }
            keep
        });
        for (k, (symbol, tf)) in wanted {
            if self.feeds.contains_key(&k) {
                continue;
            }
            let session = crate::chart::new_session_id();
            send(Request::Open {
                chart: session,
                symbol: symbol.clone(),
                tf,
                ext: false,
            });
            self.feeds.insert(
                k,
                Feed {
                    session,
                    symbol,
                    tf,
                    bars: Vec::new(),
                    version: 0,
                    resets: 0,
                },
            );
        }
        self.runs.retain(|id, _| ids.contains(id));
    }

    /// Takes the hub events meant for its sessions; hands back the rest.
    pub fn apply(&mut self, e: Event) -> Option<Event> {
        let chart = match &e {
            Event::Bars { chart, .. } | Event::Tick { chart, .. } | Event::Lot { chart, .. } => *chart,
            Event::Status { chart, .. } | Event::Corporate { chart, .. } => *chart,
            _ => return Some(e),
        };
        let Some(f) = self.feeds.values_mut().find(|f| f.session == chart) else {
            return Some(e);
        };
        match e {
            Event::Bars { bars, .. } => {
                f.bars = bars;
                f.version += 1;
                f.resets += 1;
            }
            Event::Tick { bar, .. } => {
                match f.bars.last_mut() {
                    Some(last) if last.time == bar.time => *last = bar,
                    Some(last) if last.time > bar.time => return None,
                    _ => f.bars.push(bar),
                }
                f.version += 1;
            }
            Event::Lot { min_qty, .. } => f.symbol.min_qty = min_qty,
            _ => {}
        }
        None
    }

    /// The latest price of a symbol it follows, on any interval.
    pub fn price(&self, symbol: &str) -> Option<f64> {
        self.feeds
            .values()
            .filter(|f| f.symbol.id == symbol)
            .filter_map(|f| f.bars.last())
            .max_by_key(|b| b.time)
            .map(|b| b.close)
    }

    /// A study's error, if its alert's study couldn't be built.
    pub fn error(&self, id: i64) -> Option<&str> {
        self.runs.get(&id).and_then(|r| r.error.as_deref())
    }

    /// What the alert sees now.
    pub fn probe(&mut self, a: &Alert, now: i64) -> Probe {
        let Some(f) = self.feeds.get(&feed_key(a)) else {
            return Probe::Nothing;
        };
        let Some(last) = f.bars.last() else {
            return Probe::Nothing;
        };
        let (price, bucket) = (last.close, last.time);
        match &a.condition {
            Condition::Price { value, .. } => Probe::Values(price, *value, bucket),
            Condition::Line { .. } => match a.condition.line_price(now) {
                Some(l) => Probe::Values(price, l, bucket),
                None => Probe::Nothing,
            },
            Condition::Plot { plot, value, .. } => {
                let (plot, value) = (*plot, *value);
                let Some(inst) = self.run(a) else {
                    return Probe::Nothing;
                };
                match inst.out.plots.get(plot).and_then(|p| p.last()) {
                    Some(v) => Probe::Values(*v, value.unwrap_or(price), bucket),
                    None => Probe::Nothing,
                }
            }
            Condition::Script { .. } => {
                let Some(inst) = self.run(a) else {
                    return Probe::Nothing;
                };
                script_alerts(inst).map_or(Probe::Nothing, |list| Probe::Script(list, bucket))
            }
        }
    }

    /// The alert's study, built on first use and brought up to date with
    /// its feed.
    fn run(&mut self, a: &Alert) -> Option<&Instance> {
        let spec = a.study_spec.as_ref()?;
        let fk = feed_key(a);
        let f = self.feeds.get(&fk)?;
        let key = format!("{fk}|{}|{}", spec.id, spec.inputs);
        let stale = self.runs.get(&a.id).is_none_or(|r| r.key != key);
        if stale {
            let built = build(&self.scripts, &spec.id, &spec.inputs, &f.symbol);
            let (inst, error) = match built {
                Ok(i) => (Some(i), None),
                Err(e) => {
                    crate::diag::crumb(format!("alert {}: {e}", a.id));
                    (None, Some(e))
                }
            };
            self.runs.insert(
                a.id,
                Run {
                    key,
                    inst,
                    version: 0,
                    resets: u64::MAX,
                    error,
                },
            );
        }
        let r = self.runs.get_mut(&a.id)?;
        let inst = r.inst.as_mut()?;
        if r.version != f.version {
            let tz = jiff::tz::TimeZone::get(&f.symbol.timezone).unwrap_or(jiff::tz::TimeZone::UTC);
            let s = sessions(&f.bars, f.tf, &tz);
            if r.resets != f.resets {
                inst.recompute(&f.bars, &s, f.tf);
            } else {
                inst.update(&f.bars, &s, f.tf);
            }
            r.version = f.version;
            r.resets = f.resets;
        }
        r.inst.as_ref()
    }
}

/// A built-in by id, or a script file (`pine:rsi.pine`) in `scripts`.
fn build(scripts: &std::path::Path, id: &str, inputs: &serde_json::Value, sym: &Symbol) -> Result<Instance, String> {
    let inputs: Option<Inputs> = serde_json::from_value(inputs.clone()).ok();
    if let Some(meta) = opentern_series::builtins::by_id(id) {
        let inputs = inputs
            .map(|i| i.normalized(meta))
            .unwrap_or_else(|| meta.default_inputs());
        return Ok(Instance::new(meta, &inputs));
    }
    let file = id.split_once(':').map(|(_, f)| f).unwrap_or(id);
    let inputs = inputs.unwrap_or_default();
    crate::pine_host::instance_for_file(scripts, file, sym, &inputs)
}

/// Every alert a script raised so far, from any language.
pub fn script_alerts(inst: &Instance) -> Option<Vec<(i64, String, String)>> {
    let st = inst.latest_state()?;
    if let Some(p) = st.downcast_ref::<PineStudy>() {
        Some(
            p.alerts()
                .iter()
                .map(|a| (a.time, a.message.clone(), a.title.clone()))
                .collect(),
        )
    } else {
        st.downcast_ref::<opentern_script::ScriptStudy>().map(|p| {
            p.alerts()
                .iter()
                .map(|a| (a.time, a.message.clone(), String::new()))
                .collect()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use opentern_engine::alerts::{Op, StudySpec};

    fn bar(t: i64, c: f64) -> Bar {
        Bar {
            time: t,
            open: c,
            high: c,
            low: c,
            close: c,
            volume: 1.0,
        }
    }

    #[test]
    fn a_plot_alert_runs_its_study_off_chart() {
        let mut w = Watcher::new(std::env::temp_dir());
        let a = Alert {
            id: 7,
            symbol: "BINANCE:BTCUSDT".into(),
            tf: "1h".into(),
            condition: Condition::Plot {
                uid: 1,
                study: "SMA".into(),
                plot: 0,
                title: "SMA".into(),
                op: Op::Crossing,
                value: Some(0.0),
            },
            study_spec: Some(StudySpec {
                id: "sma".into(),
                inputs: serde_json::Value::Null,
            }),
            ..Alert::default()
        };
        let sent = std::cell::RefCell::new(Vec::new());
        w.sync(std::iter::once(&a), &|r| sent.borrow_mut().push(r));
        let session = match sent.borrow().first() {
            Some(Request::Open { chart, .. }) => *chart,
            _ => panic!("no session opened"),
        };
        assert!(matches!(w.probe(&a, 0), Probe::Nothing));
        let bars: Vec<Bar> = (0..40).map(|i| bar(i * 3_600_000, 100.0 + i as f64)).collect();
        assert!(
            w.apply(Event::Bars {
                chart: session,
                bars,
                complete: true
            })
            .is_none()
        );
        let Probe::Values(v, _, t) = w.probe(&a, 0) else {
            panic!("no value")
        };
        // SMA 9 of 131..139 (the default length) is 135.
        assert!((v - 135.0).abs() < 1e-9, "{v}");
        assert_eq!(t, 39 * 3_600_000);
        // A tick moves it.
        w.apply(Event::Tick {
            chart: session,
            bar: bar(39 * 3_600_000, 148.0),
        });
        let Probe::Values(v, _, _) = w.probe(&a, 0) else {
            panic!("no value")
        };
        assert!((v - 136.0).abs() < 1e-9, "{v}");
        // Dropping the alert closes the session.
        w.sync(std::iter::empty(), &|r| sent.borrow_mut().push(r));
        assert!(matches!(sent.borrow().last(), Some(Request::Close { .. })));
    }
}

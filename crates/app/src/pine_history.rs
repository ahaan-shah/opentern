//! Prepare Pine recomputes against private chart and feed state. Replay the
//! original hub batch on the UI only after every result is ready, so the
//! frame's consumers still see a single ordered data update.

use super::App;
use crate::chart::{Chart, PreparedStudies};
use opentern_data::{Event, Hub};
use opentern_pine::{Feeds, PineStudy};
use opentern_series::study::{Factory, Inputs, Instance, Meta, Value};
use std::collections::HashMap;
use std::sync::{Arc, mpsc};
use std::time::Duration;

struct StudySpec {
    uid: u64,
    meta: &'static Meta,
    inputs: Inputs,
    factory: Factory,
    feed_rerun: bool,
}

struct ChartSpec {
    id: u64,
    version: u64,
    absorbs_history: bool,
    replaying: bool,
    studies: Vec<StudySpec>,
}

impl ChartSpec {
    fn new(chart: &Chart, scripts: &[crate::scripting::ScriptRun]) -> Self {
        let studies = chart
            .studies
            .iter()
            .filter_map(|s| {
                s.inst.state()?.downcast_ref::<PineStudy>()?;
                Some(StudySpec {
                    uid: s.uid,
                    meta: s.meta(),
                    inputs: s.inst.inputs.clone(),
                    factory: s.inst.factory()?,
                    feed_rerun: scripts.iter().any(|r| r.uid == Some(s.uid) && r.pine.is_some()),
                })
            })
            .collect();
        Self {
            id: chart.id,
            version: chart.data_version,
            absorbs_history: chart.absorbs_history(),
            replaying: chart.replaying(),
            studies,
        }
    }

    fn matches(&self, other: &Self) -> bool {
        self.id == other.id
            && self.version == other.version
            && self.absorbs_history == other.absorbs_history
            && self.replaying == other.replaying
            && self.studies.len() == other.studies.len()
            && self.studies.iter().zip(&other.studies).all(|(a, b)| {
                a.uid == b.uid
                    && Arc::ptr_eq(&a.factory, &b.factory)
                    && same_inputs(&a.inputs, &b.inputs)
                    && a.feed_rerun == b.feed_rerun
            })
    }
}

// Equality of the values as the VM receives them, including signed
// zero and defaults which may normalize to NaN.
fn same_inputs(a: &Inputs, b: &Inputs) -> bool {
    a.values.len() == b.values.len()
        && a.values.iter().zip(&b.values).all(|((ka, a), (kb, b))| {
            ka == kb
                && match (a, b) {
                    (Value::Float(a), Value::Float(b)) => a.to_bits() == b.to_bits(),
                    _ => a == b,
                }
        })
}

struct Preview {
    chart: Chart,
    spec: ChartSpec,
}

fn recompute(chart: &Chart, spec: &ChartSpec, feed_only: bool, out: &mut PreparedStudies) {
    if chart.bars.is_empty() {
        return;
    }
    for s in &spec.studies {
        if !feed_only || s.feed_rerun {
            let mut inst = Instance::with_factory(s.meta, &s.inputs, s.factory.clone());
            chart.recompute_instance(&mut inst);
            out.insert((chart.id, s.uid), inst);
        }
    }
}

#[derive(Default)]
pub(super) struct PreparedEvent {
    pub(super) studies: PreparedStudies,
    pub(super) quote_time: Option<i64>,
}

struct Finished {
    specs: Vec<ChartSpec>,
    prepared: Vec<PreparedEvent>,
}

pub(super) struct PendingBatch {
    events: Arc<Vec<Event>>,
    ready: mpsc::Receiver<std::thread::Result<Finished>>,
}

fn preview_series(e: &Event) -> Option<Event> {
    match e {
        Event::Bars { chart, bars, complete } => Some(Event::Bars {
            chart: *chart,
            bars: bars.clone(),
            complete: *complete,
        }),
        Event::Tick { chart, bar } => Some(Event::Tick {
            chart: *chart,
            bar: *bar,
        }),
        Event::Corporate { chart, corporate } => Some(Event::Corporate {
            chart: *chart,
            corporate: corporate.clone(),
        }),
        _ => None,
    }
}

fn prepare(
    events: &[Event],
    mut charts: Vec<Preview>,
    feed_keys: HashMap<u64, String>,
    feeds: &Feeds,
    demo: bool,
) -> Finished {
    let prepared = opentern_pine::with_feed_snapshot(feeds, |snapshot| {
        events
            .iter()
            .map(|e| {
                let mut out = PreparedEvent::default();
                match e {
                    Event::Quote { id, last, .. } if !demo => {
                        let now = jiff::Timestamp::now().as_millisecond();
                        out.quote_time = Some(now);
                        for p in charts.iter_mut().filter(|p| p.chart.symbol.id == *id) {
                            p.chart
                                .take_quote_with(*last, now, &mut |c| recompute(c, &p.spec, false, &mut out.studies));
                        }
                    }
                    Event::Replaced { from, to } => {
                        for p in charts.iter_mut().filter(|p| p.chart.symbol.id == from.id) {
                            p.chart.symbol = to.clone();
                        }
                    }
                    Event::Fetched { seq, result, .. } => {
                        if let Some(key) = feed_keys.get(seq)
                            && crate::feeds::replace_feed(snapshot, key, result.as_deref().unwrap_or_default().to_vec())
                        {
                            for p in &charts {
                                recompute(&p.chart, &p.spec, true, &mut out.studies);
                            }
                        }
                    }
                    Event::Bars { chart, .. } | Event::Tick { chart, .. } | Event::Corporate { chart, .. } => {
                        if let Some(p) = charts.iter_mut().find(|p| p.chart.id == *chart)
                            && let Some(event) = preview_series(e)
                        {
                            p.chart
                                .apply_with(event, &mut |c| recompute(c, &p.spec, false, &mut out.studies));
                        }
                    }
                    _ => {}
                }
                out
            })
            .collect()
    });
    let specs = charts.into_iter().map(|p| p.spec).collect();
    Finished { specs, prepared }
}

impl App {
    fn history_charts(&self) -> impl Iterator<Item = (&Chart, &[crate::scripting::ScriptRun])> {
        std::iter::once((&self.chart, self.scripts.as_slice()))
            .chain(self.slots.iter().flatten().map(|s| (&s.chart, s.scripts.as_slice())))
    }

    fn history_specs(&self) -> Vec<ChartSpec> {
        let mut specs: Vec<_> = self
            .history_charts()
            .map(|(chart, scripts)| ChartSpec::new(chart, scripts))
            .filter(|s| !s.studies.is_empty())
            .collect();
        specs.sort_unstable_by_key(|s| s.id);
        specs
    }

    pub(crate) fn history_pending(&self) -> bool {
        self.history_batch.is_some()
    }

    pub(super) fn poll_history_batch(&mut self, poll: impl FnOnce(&Hub) -> Vec<Event>) {
        let events = if let Some(batch) = self.history_batch.take() {
            match batch.ready.try_recv() {
                Err(mpsc::TryRecvError::Empty) => {
                    self.history_batch = Some(batch);
                    return;
                }
                Ok(Ok(done)) => {
                    let current = self.history_specs();
                    if current.len() == done.specs.len() && current.iter().zip(&done.specs).all(|(a, b)| a.matches(b)) {
                        let events = Arc::try_unwrap(batch.events).ok().expect("worker released the batch");
                        self.dispatch_events(events.into_iter().zip(done.prepared));
                        // New hub events may have woken frames while the batch was pending.
                        self.egui_ctx.request_repaint();
                        return;
                    }
                }
                Ok(Err(_)) | Err(mpsc::TryRecvError::Disconnected) => {
                    crate::diag::crumb("Pine history worker failed; retrying the data batch");
                }
            }
            batch.events
        } else {
            Arc::new(poll(&self.hub))
        };
        if events.is_empty() {
            return;
        }

        let feed_keys: HashMap<_, _> = events
            .iter()
            .filter_map(|e| match e {
                Event::Fetched { seq, .. } if crate::feeds::is_feed_seq(*seq) => {
                    self.feeds_state.key(*seq).map(|key| (*seq, key.to_owned()))
                }
                _ => None,
            })
            .collect();
        let pair_changed = events.iter().any(|e| matches!(e, Event::Replaced { .. }));
        let needs_worker = self.history_charts().any(|(c, _)| {
            events.iter().any(|e| match e {
                Event::Bars { chart, bars, .. } => *chart == c.id && !bars.is_empty() && !c.absorbs_history(),
                Event::Corporate { chart, .. } => *chart == c.id && c.adj,
                Event::Tick { chart, .. } => *chart == c.id && !c.absorbs_history() && c.ticks_rebuild(),
                Event::Quote { id, .. } => {
                    !self.demo && !c.replaying() && (c.symbol.id == *id || pair_changed) && c.ticks_rebuild()
                }
                Event::Fetched { seq, .. } => feed_keys.contains_key(seq),
                _ => false,
            }) && c
                .studies
                .iter()
                .any(|s| s.inst.state().is_some_and(|v| v.is::<PineStudy>()))
        });
        if !needs_worker {
            let events = Arc::try_unwrap(events).ok().expect("no worker owns the batch");
            self.dispatch_events(events.into_iter().map(|e| (e, PreparedEvent::default())));
            return;
        }

        let charts: Vec<_> = self
            .history_specs()
            .into_iter()
            .map(|spec| {
                let (chart, _) = self
                    .history_charts()
                    .find(|(c, _)| c.id == spec.id)
                    .expect("chart in snapshot");
                Preview {
                    chart: chart.history_preview(),
                    spec,
                }
            })
            .collect();
        let (tx, ready) = mpsc::channel();
        let worker_events = events.clone();
        let ctx = self.egui_ctx.clone();
        let feeds = self.feeds.clone();
        let demo = self.demo;
        let run = std::thread::Builder::new()
            .name("pine-history".into())
            .stack_size(32 << 20)
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    prepare(&worker_events, charts, feed_keys, &feeds, demo)
                }));
                drop(worker_events);
                let delay = result.is_err();
                let _ = tx.send(result);
                if delay {
                    ctx.request_repaint_after(Duration::from_millis(250));
                } else {
                    ctx.request_repaint();
                }
            });
        if let Err(e) = run {
            crate::diag::crumb(format!("couldn't start Pine history worker: {e}"));
            self.egui_ctx.request_repaint_after(Duration::from_millis(250));
        }
        self.history_batch = Some(PendingBatch { events, ready });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chart::style::SeriesKind;
    use crate::drive::Driver;
    use crate::multi::Grid;
    use opentern_core::{Bar, Timeframe};
    use opentern_data::Status;
    use std::sync::Mutex;
    use std::time::Instant;

    const FILE: &str = "history-parity.pine";
    const SOURCE: &str = r#"//@version=6
strategy("History parity", overlay=true, calc_on_order_fills=true)
n = input.int(3, "Length")
varip int seen = 0
seen += 1
float weighted = 0.0
for i = 1 to 400
    weighted += close / i
other = request.security("TEST:OTHER", "1", close)
if bar_index % 11 == 0 and strategy.position_size == 0
    strategy.entry("L", strategy.long)
if bar_index % 11 == 5 and strategy.position_size > 0
    strategy.close("L")
plot(weighted + nz(other) + n)
plot(seen)
plot(strategy.position_size)
if barstate.islast
    log.info(str.tostring(seen))
"#;

    fn bar(i: i64, offset: f64) -> Bar {
        let price = 100.0 + (i % 13) as f64 + offset;
        Bar {
            time: 1_700_000_000_000 + i * 60_000,
            open: price,
            high: price + 1.0,
            low: price - 1.0,
            close: price + 0.5,
            volume: 10.0,
        }
    }

    fn history(id: u64, start: i64, end: i64) -> Event {
        Event::Bars {
            chart: id,
            bars: (start..end).map(|i| bar(i, 0.0)).collect(),
            complete: true,
        }
    }

    fn fetched(offset: f64) -> Event {
        Event::Fetched {
            seq: crate::feeds::SEQ_BASE + 1,
            symbol: "TEST:OTHER".into(),
            tf: Timeframe::parse("1m").unwrap(),
            result: Ok((0..5_010).map(|i| bar(i, offset)).collect()),
        }
    }

    fn setup(name: &str, multi: bool) -> Driver {
        let mut d = Driver::new(name);
        let app = &mut d.app;
        if multi {
            app.set_grid(Grid::TwoCols);
        }
        let dir = app.scripts_dir();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(FILE), SOURCE).unwrap();
        app.each_chart(|a| {
            let c = &mut a.chart;
            let demo_studies: Vec<_> = c.studies.iter().map(|s| s.uid).collect();
            for uid in demo_studies {
                c.remove_study(uid);
            }
            c.reset_series(c.symbol.clone(), Timeframe::parse("1m").unwrap());
            c.apply(history(c.id, 40, 48));
            a.add_script(FILE);
            assert!(a.scripts[0].compile_error.is_none(), "{:?}", a.scripts[0].error);
            assert_eq!(a.chart.studies.len(), 1);
        });
        app.request_feeds();
        assert_eq!(app.feeds_state.key(crate::feeds::SEQ_BASE + 1), Some("TEST:OTHER|1m"));
        d
    }

    fn configure_second(a: &mut App) {
        a.with_slot(1, |a| {
            a.chart.style.kind = SeriesKind::Renko;
            a.chart.style.box_size = Some(1.0);
            a.chart.style_changed();
            a.chart.set_adj(true);
        })
        .unwrap();
    }

    fn sequence(a: &App) -> Vec<Event> {
        let main = a.chart.id;
        let second = &a.slots[1].as_ref().unwrap().chart;
        let late = Bar {
            time: 2_000_000_000_000,
            ..bar(87, 6.0)
        };
        vec![
            history(main, 0, 5_000),
            Event::Quote {
                id: "TEST:ATOMIC".into(),
                last: 11.0,
                prev: 10.0,
            },
            Event::Tick {
                chart: main,
                bar: bar(4_999, 1.0),
            },
            fetched(50.0),
            history(second.id, 0, 87),
            Event::Tick {
                chart: second.id,
                bar: late,
            },
            Event::Quote {
                id: second.symbol.id.clone(),
                last: 130.0,
                prev: 120.0,
            },
            Event::Corporate {
                chart: second.id,
                corporate: Default::default(),
            },
            fetched(51.0),
            fetched(51.0),
            Event::Tick {
                chart: main,
                bar: bar(4_999, 2.0),
            },
            Event::Tick {
                chart: main,
                bar: bar(5_000, 0.0),
            },
            Event::Tick {
                chart: main,
                bar: bar(5_000, 3.0),
            },
            Event::Status {
                chart: main,
                status: Status::Live,
            },
        ]
    }

    fn sync(a: &mut App, events: Vec<Event>) {
        a.dispatch_events(events.into_iter().map(|e| (e, PreparedEvent::default())));
    }

    fn complete(a: &mut App) {
        let deadline = Instant::now() + Duration::from_secs(20);
        while a.history_pending() {
            assert!(Instant::now() < deadline, "data batch did not finish");
            a.poll_history_batch(|_| panic!("cannot drain more events while a batch is pending"));
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    fn plots(a: &Chart) -> Vec<Vec<u64>> {
        a.studies[0]
            .inst
            .out
            .plots
            .iter()
            .map(|v| v.iter().map(|f| f.to_bits()).collect())
            .collect()
    }

    fn compare(a: &App, expected: &App) {
        let (c, e) = (&a.chart, &expected.chart);
        assert_eq!(c.bars, e.bars);
        assert_eq!(c.data_version, e.data_version);
        assert_eq!(c.changed, e.changed);
        assert_eq!(c.complete, e.complete);
        assert_eq!(c.status, e.status);
        assert_eq!(plots(c), plots(e));
        for forming in [false, true] {
            let actual = crate::strategy::closed_broker(&c.studies[0].inst, &c.bars, forming).unwrap();
            let expected = crate::strategy::closed_broker(&e.studies[0].inst, &e.bars, forming).unwrap();
            assert!(!actual.fills.is_empty());
            assert_eq!(actual.fills, expected.fills);
            assert_eq!(actual.trades, expected.trades);
            assert_eq!(actual.equity, expected.equity);
            // Undefined ratios are NaN in both reports. Compare their
            // representations so matching NaNs don't fail the oracle.
            let actual = opentern_engine::Report::new(&actual, &c.bars);
            let expected = opentern_engine::Report::new(&expected, &e.bars);
            assert_eq!(format!("{actual:?}"), format!("{expected:?}"));
        }
        fn state(c: &Chart) -> &PineStudy {
            c.studies[0]
                .inst
                .latest_state()
                .unwrap()
                .downcast_ref::<PineStudy>()
                .unwrap()
        }
        assert_eq!(state(c).logs(), state(e).logs());
        assert_eq!(state(c).wanted(), state(e).wanted());
        assert_eq!(state(c).error(), state(e).error());
    }

    #[test]
    fn chart_and_feed_history_leave_the_frame_thread_and_publish_in_order() {
        let mut a = setup("history-candidate", true);
        let mut reference = setup("history-sync", true);
        configure_second(&mut a.app);
        configure_second(&mut reference.app);
        a.app.demo = false;
        reference.app.demo = false;
        let (calls, received) = mpsc::channel();
        let (release, gate) = mpsc::channel();
        let gate = Arc::new(Mutex::new(Some(gate)));
        a.app.each_chart(|a| {
            let s = &a.chart.studies[0];
            let uid = s.uid;
            let factory = s.inst.factory().unwrap();
            let input = s.inst.inputs.clone();
            let meta = s.meta();
            let (calls, gate) = (calls.clone(), gate.clone());
            let trace: Factory = Arc::new(move |i| {
                let t = std::thread::current();
                if t.name() == Some("pine-history") {
                    calls.send((t.id(), Instant::now())).unwrap();
                    if let Some(g) = gate.lock().unwrap().take() {
                        g.recv_timeout(Duration::from_secs(15)).unwrap();
                    }
                } else {
                    calls.send((t.id(), Instant::now())).unwrap();
                }
                factory(i)
            });
            a.chart
                .replace_instance(uid, Instance::with_factory(meta, &input, trace));
        });
        while received.try_recv().is_ok() {}
        let expected_events = sequence(&reference.app);
        let events = sequence(&a.app);
        let ui = std::thread::current().id();
        let old_bars = a.app.chart.bars.clone();
        let old_plots = plots(&a.app.chart);
        let start = Instant::now();
        a.app.poll_history_batch(|_| events);
        let submission = start.elapsed();
        let (worker, _) = received.recv_timeout(Duration::from_secs(10)).unwrap();
        assert_ne!(ui, worker);
        assert!(a.app.history_pending());
        assert_eq!(a.app.chart.bars, old_bars);
        assert_eq!(plots(&a.app.chart), old_plots);
        assert!(a.app.feeds.read().unwrap().is_empty());
        assert!(!a.app.quotes.contains_key("TEST:ATOMIC"));

        // An actual egui frame completes while the worker is held before
        // its first Pine recompute. Switching the active slot is harmless.
        let frame = Instant::now();
        a.frame(Vec::new());
        let frame_time = frame.elapsed();
        assert_eq!(a.app.chart.bars, old_bars);
        assert_eq!(plots(&a.app.chart), old_plots);
        assert!(a.app.feeds.read().unwrap().is_empty());
        a.app.activate(1);
        let release_at = Instant::now();
        release.send(()).unwrap();
        complete(&mut a.app);
        let elapsed = release_at.elapsed();
        let builds: Vec<_> = received.try_iter().collect();
        // main Bars, two changed feeds for both charts, second Bars,
        // transformed Tick, Quote and Corporate = nine recomputes, each with two factory builds
        // (the fresh instance and Instance::recompute's reset).
        assert_eq!(builds.len() + 1, 18);
        assert!(builds.iter().all(|(t, _)| *t == worker));
        eprintln!(
            "history parity ui={ui:?} worker={worker:?} bars=5000 whole_recomputes=9              submit_us={} frame_while_worker_blocked_us={} worker_and_commit_after_release_us={}",
            submission.as_micros(),
            frame_time.as_micros(),
            elapsed.as_micros()
        );

        sync(&mut reference.app, expected_events);
        reference.app.activate(1);
        compare(&a.app, &reference.app);
        a.app.activate(0);
        reference.app.activate(0);
        compare(&a.app, &reference.app);
        assert_eq!(*a.app.feeds.read().unwrap(), *reference.app.feeds.read().unwrap());
        assert_eq!(a.app.quotes.get("TEST:ATOMIC"), Some(&(11.0, 10.0)));
    }

    #[test]
    fn a_replaced_study_and_closed_session_cannot_receive_old_results() {
        let mut a = setup("history-stale", true);
        let mut expected = setup("history-stale-reference", true);
        let batch = |app: &App| {
            vec![
                history(app.chart.id, 0, 120),
                history(app.slots[1].as_ref().unwrap().chart.id, 0, 200),
                fetched(50.0),
            ]
        };
        let events = batch(&a.app);
        let expected_events = batch(&expected.app);
        let old = plots(&a.app.chart);
        a.app.poll_history_batch(|_| events);
        assert!(a.app.history_pending());
        for d in [&mut a, &mut expected] {
            let app = &mut d.app;
            app.set_grid(Grid::One);
            let previous = app.chart.studies[0].inst.inputs.clone();
            let mut edited = previous.clone();
            let key = edited.values.keys().next().unwrap().clone();
            edited.set(&key, Value::Int(17));
            app.chart.set_inputs(app.chart.studies[0].uid, &edited);
            assert_ne!(edited, previous);
            app.chart.toggle_replay();
            app.chart.replay_from(4);
        }
        assert_ne!(plots(&a.app.chart), old);
        assert!(a.app.feeds.read().unwrap().is_empty());
        complete(&mut a.app);
        sync(&mut expected.app, expected_events);
        compare(&a.app, &expected.app);
        assert_eq!(a.app.chart_count(), 1);
        assert_eq!(a.app.chart.bars.len(), 5);
        assert_eq!(*a.app.feeds.read().unwrap(), *expected.app.feeds.read().unwrap());
        a.app.chart.exit_replay();
        expected.app.chart.exit_replay();
        assert_eq!(a.app.chart.bars.len(), 120);
        compare(&a.app, &expected.app);
        eprintln!("stale batch retried after input change, chart close and replay start; hidden history preserved");
    }

    #[test]
    fn worker_failure_preserves_the_batch_for_retry() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let mut a = setup("history-retry", false);
        let c = &mut a.app.chart;
        let s = &c.studies[0];
        let (id, uid) = (c.id, s.uid);
        let inner = s.inst.factory().unwrap();
        let failed = Arc::new(AtomicBool::new(false));
        let state = failed.clone();
        let factory: Factory = Arc::new(move |inputs| {
            if std::thread::current().name() == Some("pine-history") && !state.swap(true, Ordering::Relaxed) {
                panic!("injected failure before Pine history computation");
            }
            inner(inputs)
        });
        let inst = Instance::with_factory(s.meta(), &s.inst.inputs, factory);
        c.replace_instance(uid, inst);
        let version = c.data_version;
        a.app.poll_history_batch(|_| vec![history(id, 0, 120), fetched(50.0)]);
        assert_eq!(a.app.chart.data_version, version);
        assert!(a.app.feeds.read().unwrap().is_empty());
        complete(&mut a.app);
        assert!(failed.load(Ordering::Relaxed));
        assert_eq!(a.app.chart.data_version, version + 1);
        assert_eq!(a.app.chart.bars.len(), 120);
        assert_eq!(a.app.feeds.read().unwrap().len(), 1);
        eprintln!("injected worker failure retried: chart history and feed committed once");
    }
}

//! Drives the real app headlessly: frames run in an egui context fed with
//! synthetic pointer and key events, against made-up (demo) data. Used to
//! test interaction (drawings, dialogs) the way a person would use it, and
//! for a random-input soak test.

use crate::app::App;
use crate::chart::draw::Tool;
use egui::{Event, Key, Modifiers, PointerButton, Pos2, RawInput, Rect, pos2, vec2};
use std::time::Duration;

pub struct Driver {
    pub ctx: egui::Context,
    pub app: App,
    time: f64,
    pointer: Pos2,
    pub dir: std::path::PathBuf,
}

impl Driver {
    pub fn new(name: &str) -> Driver {
        let dir = std::env::temp_dir().join(format!("opentern-drive-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        Driver::open(dir)
    }

    pub fn open(dir: std::path::PathBuf) -> Driver {
        // SAFETY: tests in this binary share these values and never remove them.
        unsafe {
            std::env::set_var("OPENTERN_HEADLESS", "1");
        }
        let store = opentern_core::Store::open(&dir).expect("open store");
        let ctx = egui::Context::default();
        let app = App::with_context(&ctx, Default::default(), store, true);
        let mut d = Driver {
            ctx,
            app,
            time: 0.0,
            pointer: pos2(10.0, 10.0),
            dir,
        };
        d.wait(|a| !a.chart.bars.is_empty());
        d
    }

    pub fn frame(&mut self, events: Vec<Event>) {
        self.time += 1.0 / 60.0;
        let raw = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1400.0, 900.0))),
            time: Some(self.time),
            events,
            ..Default::default()
        };
        let app = &mut self.app;
        let mut out = self.ctx.run_ui(raw, |ui| app.frame(ui));
        out.textures_delta.clear();
    }

    /// Runs frames until `f` holds (data arrives on other threads).
    pub fn wait(&mut self, f: impl Fn(&App) -> bool) {
        for _ in 0..400 {
            self.frame(vec![]);
            if f(&self.app) {
                // A couple more so layout settles.
                self.frame(vec![]);
                self.frame(vec![]);
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("timed out waiting");
    }

    pub fn move_to(&mut self, p: Pos2) {
        self.pointer = p;
        self.frame(vec![Event::PointerMoved(p)]);
    }

    fn button(p: Pos2, pressed: bool) -> Event {
        Event::PointerButton {
            pos: p,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        }
    }

    pub fn click(&mut self, p: Pos2) {
        self.move_to(p);
        self.frame(vec![Self::button(p, true)]);
        self.frame(vec![Self::button(p, false)]);
        self.frame(vec![]);
    }

    pub fn drag(&mut self, from: Pos2, to: Pos2) {
        self.move_to(from);
        self.frame(vec![Self::button(from, true)]);
        for k in 1..=8 {
            let p = from + (to - from) * (k as f32 / 8.0);
            self.frame(vec![Event::PointerMoved(p)]);
        }
        self.frame(vec![Self::button(to, false)]);
        self.frame(vec![]);
    }

    pub fn key(&mut self, key: Key, modifiers: Modifiers) {
        let ev = |pressed| Event::Key {
            key,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers,
        };
        self.frame(vec![ev(true)]);
        self.frame(vec![ev(false)]);
    }

    pub fn scroll(&mut self, at: Pos2, dy: f32) {
        self.move_to(at);
        for _ in 0..10 {
            self.frame(vec![Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: vec2(0.0, dy),
                modifiers: Modifiers::NONE,
                phase: egui::TouchPhase::Move,
            }]);
        }
    }

    /// A point in the main chart pane, as fractions of its size.
    pub fn at(&self, fx: f32, fy: f32) -> Pos2 {
        let r = self.app.chart_ui.main_rect;
        pos2(r.left() + r.width() * fx, r.top() + r.height() * fy)
    }

    pub fn pick(&mut self, tool: Tool) {
        self.app.pick_tool(tool);
        self.frame(vec![]);
    }
}

impl Drop for Driver {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn place_move_undo_delete_a_trend_line() {
        let mut d = Driver::new("trend");
        assert!(d.app.chart_ui.main_rect.width() > 100.0, "chart laid out");
        d.pick(Tool::TrendLine);
        d.click(d.at(0.3, 0.5));
        d.click(d.at(0.6, 0.3));
        assert_eq!(d.app.chart.drawings.len(), 1, "placed");
        assert_eq!(d.app.layout.tool, Tool::Cross, "back to the cross");
        let before = d.app.chart.drawings[0].clone();
        assert_ne!(before.pts[0], before.pts[1]);
        // Drag its middle: both points move.
        let mid = d.at(0.45, 0.4);
        d.drag(mid, mid + vec2(60.0, 40.0));
        let moved = d.app.chart.drawings[0].clone();
        assert_ne!(moved.pts[0], before.pts[0], "moved");
        assert!(moved.pts[0].p < before.pts[0].p, "dragged down means lower price");
        d.key(Key::Z, Modifiers::COMMAND);
        assert_eq!(d.app.chart.drawings[0], before, "undo");
        d.key(Key::Y, Modifiers::COMMAND);
        assert_eq!(d.app.chart.drawings[0], moved, "redo");
        // Select and delete.
        d.click(d.at(0.45, 0.4) + vec2(60.0, 40.0));
        assert!(d.app.chart_ui.draw.selected.is_some(), "selected");
        d.key(Key::Delete, Modifiers::NONE);
        assert!(d.app.chart.drawings.is_empty(), "deleted");
        assert_eq!(d.app.panics, 0);
    }

    #[test]
    fn one_click_tools_and_drawings_persist_per_symbol() {
        let mut d = Driver::new("persist");
        for tool in [Tool::HorizontalLine, Tool::LongPosition, Tool::Text, Tool::VerticalLine] {
            d.pick(tool);
            d.click(d.at(0.5, 0.5));
        }
        assert_eq!(d.app.chart.drawings.len(), 4);
        let long = d
            .app
            .chart
            .drawings
            .iter()
            .find(|x| x.tool == Tool::LongPosition)
            .unwrap();
        assert_eq!(long.pts.len(), 3);
        let symbol = d.app.chart.symbol.id.clone();
        // Another symbol has none; coming back brings them back.
        let eth = opentern_core::starter_watchlist().remove(1);
        d.app.open_symbol(eth);
        d.wait(|a| !a.chart.bars.is_empty());
        assert!(d.app.chart.drawings.is_empty());
        let btc = opentern_core::starter_watchlist()
            .into_iter()
            .find(|s| s.id == symbol)
            .unwrap();
        d.app.open_symbol(btc);
        d.wait(|a| !a.chart.bars.is_empty());
        assert_eq!(d.app.chart.drawings.len(), 4);
        // And they survive a restart.
        let dir = d.dir.clone();
        d.app.on_exit_for_tests();
        std::mem::forget(d);
        let mut again = Driver::open(dir);
        again.wait(|a| !a.chart.drawings.is_empty());
        assert_eq!(again.app.chart.drawings.len(), 4);
    }

    #[test]
    fn multi_chart_layouts_sync_and_survive_a_restart() {
        use crate::multi::Grid;
        let mut d = Driver::new("multi");
        d.app.set_grid(Grid::Four);
        d.wait(|a| a.slots.iter().flatten().all(|s| !s.chart.bars.is_empty()));
        assert_eq!(d.app.chart_count(), 4);
        // Every cell paints: the main rects of all four charts are laid out.
        d.frame(vec![]);
        let rects: Vec<Rect> = (0..4)
            .map(|i| d.app.with_slot(i, |a| a.chart_ui.main_rect).unwrap())
            .collect();
        assert!(rects.iter().all(|r| r.width() > 100.0), "{rects:?}");
        // Pressing in the bottom-right chart makes it active.
        let r = rects[3];
        d.click(r.center());
        assert_eq!(d.app.active, 3);
        // Symbol not synced: only the active chart changes.
        let eth = opentern_core::starter_watchlist().remove(1);
        d.app.open_symbol_synced(eth.clone());
        d.wait(|a| !a.chart.bars.is_empty());
        let syms: Vec<String> = (0..4)
            .map(|i| d.app.with_slot(i, |a| a.chart.symbol.id.clone()).unwrap())
            .collect();
        assert_eq!(syms.iter().filter(|s| **s == eth.id).count(), 1);
        // Synced: all of them.
        d.app.multi.sync.symbol = true;
        d.app.open_symbol_synced(eth.clone());
        d.frame(vec![]);
        for i in 0..4 {
            assert_eq!(d.app.with_slot(i, |a| a.chart.symbol.id.clone()).unwrap(), eth.id);
        }
        // A drawing on one chart shows on the others with the same symbol.
        d.wait(|a| !a.chart.bars.is_empty());
        d.pick(Tool::HorizontalLine);
        d.click(d.at(0.5, 0.5));
        d.app.save_drawings_now();
        assert_eq!(d.app.with_slot(0, |a| a.chart.drawings.len()).unwrap(), 1);
        // Restart: grid, active chart and symbols come back.
        let dir = d.dir.clone();
        d.app.on_exit_for_tests();
        std::mem::forget(d);
        let again = Driver::open(dir);
        assert_eq!(again.app.multi.grid, Grid::Four);
        assert_eq!(again.app.active, 3);
        assert!(again.app.multi.sync.symbol);
        assert_eq!(again.app.chart.symbol.id, eth.id);
        assert_eq!(again.app.panics, 0);
    }

    #[test]
    fn bar_replay_from_a_clicked_bar() {
        let mut d = Driver::new("replay");
        let n = d.app.chart.bars.len();
        d.app.chart.toggle_replay();
        d.frame(vec![]);
        d.click(d.at(0.5, 0.5));
        let cut = d.app.chart.bars.len();
        assert!(cut < n && cut > 10, "cut at {cut} of {n}");
        assert!(d.app.chart.replay.as_ref().is_some_and(|r| !r.selecting));
        assert!(d.app.chart.replay_step());
        assert_eq!(d.app.chart.bars.len(), cut + 1);
        d.app.chart.exit_replay();
        assert!(d.app.chart.bars.len() >= n);
        assert_eq!(d.app.panics, 0);
    }

    #[test]
    fn compared_symbols_load_and_are_saved() {
        let mut d = Driver::new("compare");
        let eth = opentern_core::starter_watchlist().remove(1);
        d.app.add_compare(eth.clone(), false);
        d.wait(|a| a.chart.compares.first().is_some_and(|c| !c.bars.is_empty()));
        assert!(d.app.chart.percent, "comparing switches to percent");
        let dir = d.dir.clone();
        d.app.on_exit_for_tests();
        std::mem::forget(d);
        let mut again = Driver::open(dir);
        again.wait(|a| a.chart.compares.first().is_some_and(|c| !c.bars.is_empty()));
        assert_eq!(again.app.chart.compares[0].symbol.id, eth.id);
        again.app.remove_compare(0);
        assert!(!again.app.chart.percent);
    }

    #[test]
    fn scrolling_in_a_dialog_leaves_the_chart_alone() {
        let mut d = Driver::new("modal-scroll");
        let before = (d.app.chart.view.bar_w, d.app.chart.view.right);
        d.app.modal = Some(crate::dialogs::Modal::Indicators(crate::dialogs::Indicators::new(
            false,
        )));
        d.frame(vec![]);
        d.scroll(d.at(0.5, 0.5), -40.0);
        assert_eq!((d.app.chart.view.bar_w, d.app.chart.view.right), before);
        // Without the dialog, the same scroll zooms.
        d.app.modal = None;
        d.scroll(d.at(0.5, 0.5), -40.0);
        assert_ne!(d.app.chart.view.bar_w, before.0);
    }

    #[test]
    fn zoom_tool_fits_the_dragged_range() {
        let mut d = Driver::new("zoom");
        let w0 = d.app.chart.view.bar_w;
        d.pick(Tool::Zoom);
        d.drag(d.at(0.4, 0.5), d.at(0.6, 0.5));
        assert!(d.app.chart.view.bar_w > w0 * 3.0, "{} vs {w0}", d.app.chart.view.bar_w);
        assert_eq!(d.app.layout.tool, Tool::Cross);
    }

    #[test]
    fn python_script_plots_on_the_chart_and_matches_the_builtin() {
        if opentern_script::python::interpreter().is_none() {
            eprintln!("no Python; skipping");
            return;
        }
        let mut d = Driver::new("python");
        let dir = d.app.scripts_dir();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("rsi.py"),
            "from opentern import *\nindicator('Py RSI')\nn = input.int(14, 'Length')\n\
             def compute(bars):\n    plot(ta.rsi(bars.close, n), 'RSI', color='#7E57C2')\n    hline(70)\n",
        )
        .unwrap();
        d.app.add_script("rsi.py");
        d.wait(|a| {
            a.chart
                .studies
                .iter()
                .any(|s| s.meta().id == "py:rsi.py" && s.inst.out.plots[0].iter().any(|v| v.is_finite()))
        });
        // The SDK's RSI is written separately from the Rust one; they agree.
        let rsi = opentern_series::builtins::by_id("rsi").unwrap();
        let mut native = opentern_series::Instance::new(rsi, &rsi.default_inputs());
        let sess = opentern_series::study::sessions(&d.app.chart.bars, d.app.chart.tf, &jiff::tz::TimeZone::UTC);
        native.recompute(&d.app.chart.bars, &sess, d.app.chart.tf);
        let py = d.app.chart.studies.iter().find(|s| s.meta().id == "py:rsi.py").unwrap();
        // Compare the bars the script saw (it may lag a live tick).
        let n = py.inst.out.plots[0].iter().rposition(|v| v.is_finite()).unwrap();
        for i in 0..n {
            let (a, b) = (py.inst.out.plots[0][i], native.out.plots[0][i]);
            assert!(
                (a.is_nan() && b.is_nan()) || (a - b).abs() < 1e-9,
                "bar {i}: {a} vs {b}"
            );
        }
        assert_eq!(py.meta().placement, opentern_series::study::Placement::Pane);
        assert_eq!(py.meta().hlines.len(), 1);
        // Saved in the layout and restored with it.
        let state = d.app.chart.state();
        assert!(state.studies.iter().any(|s| s.id == "py:rsi.py"));
        assert_eq!(d.app.panics, 0);
    }

    #[test]
    fn pine_script_plots_reloads_and_reports_errors() {
        let mut d = Driver::new("pine");
        let dir = d.app.scripts_dir();
        std::fs::create_dir_all(&dir).unwrap();
        let src = "//@version=5\nindicator(\"Pine RSI\")\nlen = input.int(14, \"Length\")\nplot(ta.rsi(close, len), \"RSI\")\nhline(70)\n";
        std::fs::write(dir.join("rsi.pine"), src).unwrap();
        d.app.add_script("rsi.pine");
        let id = "pine:rsi.pine";
        d.wait(|a| a.chart.studies.iter().any(|s| s.meta().id == id));
        let rsi = opentern_series::builtins::by_id("rsi").unwrap();
        let mut native = opentern_series::Instance::new(rsi, &rsi.default_inputs());
        let sess = opentern_series::study::sessions(&d.app.chart.bars, d.app.chart.tf, &d.app.chart.sym_tz);
        native.recompute(&d.app.chart.bars, &sess, d.app.chart.tf);
        let pine = d.app.chart.studies.iter().find(|s| s.meta().id == id).unwrap();
        assert_eq!(pine.meta().placement, opentern_series::study::Placement::Pane);
        let n = d.app.chart.bars.len();
        for i in 0..n {
            let (a, b) = (pine.inst.out.plots[0][i], native.out.plots[0][i]);
            assert!(
                (a.is_nan() && b.is_nan()) || (a - b).abs() < 1e-9,
                "bar {i}: {a} vs {b}"
            );
        }
        // Edited in another editor: the chart picks it up.
        std::fs::write(dir.join("rsi.pine"), src.replace("input.int(14", "input.int(7")).unwrap();
        d.wait(|a| {
            a.chart
                .studies
                .iter()
                .find(|s| s.meta().id == id)
                .is_some_and(|s| s.inst.inputs.values.get("length") == Some(&opentern_series::study::Value::Int(14)))
                && a.scripts.iter().any(|r| {
                    r.pine
                        .as_ref()
                        .is_some_and(|p| matches!(p.meta().inputs[0].default, opentern_series::study::Value::Int(7)))
                })
        });
        // A syntax error keeps the last good version and says where.
        std::fs::write(
            dir.join("rsi.pine"),
            "//@version=5\nindicator(\"Pine RSI\")\nplot(ta.rsi(close, 14)\n",
        )
        .unwrap();
        d.wait(|a| a.chart.study_errors.values().any(|e| e.starts_with("Line 3")));
        assert!(d.app.chart.studies.iter().any(|s| s.meta().id == id));
        // Saved with the layout.
        assert!(d.app.chart.state().studies.iter().any(|s| s.id == id));
        assert_eq!(d.app.panics, 0);
    }

    #[test]
    fn strategy_report_and_trade_marks() {
        let mut d = Driver::new("strategy");
        let dir = d.app.scripts_dir();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("cross.pine"),
            "//@version=5\nstrategy(\"Cross\", overlay = true)\nf = ta.sma(close, 5)\ns = ta.sma(close, 20)\n\
             if ta.crossover(f, s)\n    strategy.entry(\"L\", strategy.long)\nif ta.crossunder(f, s)\n    strategy.close(\"L\")\n",
        )
        .unwrap();
        d.app.add_script("cross.pine");
        d.wait(|a| {
            a.strategies
                .runs
                .iter()
                .any(|r| r.name == "Cross" && !r.broker.trades.is_empty())
        });
        let run = &d.app.strategies.runs[0];
        assert_eq!(run.report.all.total_trades, run.broker.trades.len());
        let marks = d.app.chart.marks.clone().expect("trade marks");
        assert!(marks.fills.len() >= 2 * run.broker.trades.len());
        // The report tab draws without trouble on every sub-tab.
        d.app.layout.tab = Some(crate::shell::Tab::Report);
        for tab in [
            crate::strategy::ReportTab::Overview,
            crate::strategy::ReportTab::Performance,
            crate::strategy::ReportTab::Trades,
            crate::strategy::ReportTab::Ratios,
            crate::strategy::ReportTab::List,
            crate::strategy::ReportTab::Properties,
        ] {
            d.app.strategies.tab = tab;
            for _ in 0..3 {
                d.frame(vec![]);
            }
        }
        assert_eq!(d.app.panics, 0);
    }

    #[test]
    fn paper_orders_fill_and_survive_a_restart() {
        use opentern_engine::paper::{Kind, Ticket};
        let mut d = Driver::new("paper");
        let sym = d.app.chart.symbol.id.clone();
        let last = d.app.last_price(&sym).expect("a price");
        let ticket = Ticket {
            symbol: sym.clone(),
            buy: true,
            qty: 2.0,
            kind: Kind::Market,
            take_profit: Some(last * 1.5),
            stop_loss: Some(last * 0.5),
        };
        d.app.paper.account.place(ticket, Some(last), 0).unwrap();
        d.app.layout.tab = Some(crate::shell::Tab::Paper);
        for _ in 0..5 {
            d.frame(vec![]);
        }
        assert_eq!(d.app.paper.account.position(&sym).unwrap().qty, 2.0);
        assert_eq!(d.app.paper.account.orders.len(), 2, "take-profit and stop-loss");
        assert!(d.app.chart.paper.len() >= 3, "position and both brackets on the chart");
        // Saved when something happens in the tab; force one save, reopen.
        d.app.paper.account.cancel(d.app.paper.account.orders[0].id);
        let json = serde_json::to_string(&d.app.paper.account).unwrap();
        d.app.store.set_value("paper_account", &json).unwrap();
        let path = d.dir.clone();
        std::mem::forget(d);
        let d = Driver::open(path);
        assert_eq!(d.app.paper.account.position(&sym).unwrap().qty, 2.0);
        assert_eq!(d.app.paper.account.orders.len(), 1);
    }

    #[test]
    fn price_alerts_fire_once_and_are_logged() {
        use opentern_engine::alerts::{Alert, Condition, Freq, Op};
        let mut d = Driver::new("alerts");
        let sym = d.app.chart.symbol.id.clone();
        let last = d.app.last_price(&sym).unwrap();
        d.app.save_alert(Alert {
            name: "Above".into(),
            symbol: sym.clone(),
            condition: Condition::Price {
                op: Op::GreaterThan,
                value: last * 0.5,
            },
            freq: Freq::Once,
            ..Alert::default()
        });
        // Never fires: the price is far below.
        d.app.save_alert(Alert {
            name: "Far".into(),
            symbol: sym.clone(),
            condition: Condition::Price {
                op: Op::LessThan,
                value: last * 0.1,
            },
            freq: Freq::OncePerBar,
            ..Alert::default()
        });
        for _ in 0..3 {
            d.frame(vec![]);
        }
        assert_eq!(d.app.alerts.log.len(), 1);
        assert_eq!(d.app.alerts.log[0].name, "Above");
        assert!(!d.app.alerts.list[0].0.active, "only once: off after firing");
        assert!(d.app.alerts.list[1].0.active);
        assert_eq!(d.app.alerts.toasts.len(), 1);
        assert_eq!(d.app.store.alert_log(10).len(), 1);
        // Alt+A opens the dialog (real keyboards also send the letter as
        // text, which must not start a symbol search).
        let ev = |pressed| Event::Key {
            key: Key::A,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers: Modifiers::ALT,
        };
        d.frame(vec![
            Event::ModifiersChanged(Modifiers::ALT),
            ev(true),
            Event::Text("a".into()),
        ]);
        d.frame(vec![ev(false), Event::ModifiersChanged(Modifiers::NONE)]);
        assert!(matches!(d.app.modal, Some(crate::dialogs::Modal::Alert(_))));
        assert_eq!(d.app.panics, 0);
    }

    #[test]
    fn script_alerts_fire_for_new_bars_only() {
        use opentern_engine::alerts::{Alert, Condition, Freq};
        let mut d = Driver::new("script-alerts");
        let dir = d.app.scripts_dir();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("ping.pine"),
            "//@version=5\nindicator(\"Ping\")\nalert(\"bar \" + str.tostring(bar_index))\nplot(close)\n",
        )
        .unwrap();
        d.app.add_script("ping.pine");
        d.wait(|a| a.chart.studies.iter().any(|s| s.meta().id == "pine:ping.pine"));
        let uid = d
            .app
            .chart
            .studies
            .iter()
            .find(|s| s.meta().id == "pine:ping.pine")
            .unwrap()
            .uid;
        d.app.save_alert(Alert {
            symbol: d.app.chart.symbol.id.clone(),
            condition: Condition::Script {
                uid,
                study: "Ping".into(),
                title: None,
            },
            freq: Freq::OncePerBar,
            ..Alert::default()
        });
        for _ in 0..3 {
            d.frame(vec![]);
        }
        let before = d.app.alerts.log.len();
        assert_eq!(before, 0, "history doesn't fire");
        // A new bar arrives.
        let mut b = *d.app.chart.bars.last().unwrap();
        b.time += d.app.chart.tf.millis().unwrap();
        let id = d.app.chart.id;
        d.app.chart.apply(opentern_data::Event::Tick { chart: id, bar: b });
        for _ in 0..3 {
            d.frame(vec![]);
        }
        assert!(
            d.app.alerts.log.iter().any(|f| f.message.starts_with("bar ")),
            "{:?}",
            d.app.alerts.log
        );
        assert_eq!(d.app.panics, 0);
    }

    /// Random clicks, drags, keys and scrolls for a while: the app must
    /// never panic (the crash shield counts any that happen).
    #[test]
    fn monkey() {
        let mut d = Driver::new("monkey");
        let mut seed: u64 = 0x9E37_79B9_7F4A_7C15;
        let mut rnd = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        let frames: usize = std::env::var("MONKEY_FRAMES")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(600);
        let tools = [
            Tool::TrendLine,
            Tool::Rectangle,
            Tool::FibRetracement,
            Tool::ParallelChannel,
            Tool::Brush,
            Tool::Cross,
        ];
        for _ in 0..frames {
            let p = pos2((rnd() % 1400) as f32, (rnd() % 900) as f32);
            match rnd() % 10 {
                0..=2 => d.click(p),
                3 => d.drag(p, p + vec2((rnd() % 200) as f32 - 100.0, (rnd() % 200) as f32 - 100.0)),
                4 => d.scroll(p, (rnd() % 80) as f32 - 40.0),
                5 => d.pick(tools[(rnd() % tools.len() as u64) as usize]),
                6 => {
                    let keys = [
                        Key::Escape,
                        Key::Delete,
                        Key::ArrowLeft,
                        Key::ArrowRight,
                        Key::Z,
                        Key::Enter,
                    ];
                    d.key(keys[(rnd() % keys.len() as u64) as usize], Modifiers::NONE);
                }
                7 => d.frame(vec![Event::Text("b".into())]),
                _ => d.move_to(p),
            }
        }
        assert_eq!(d.app.panics, 0, "the app panicked during random input");
    }
}

//! Bar replay, as on TradingView: pick a bar, the chart cuts back to it,
//! then steps forward one bar at a time (by hand or playing at a speed).
//! Studies and strategies see only the bars replayed so far, so a
//! strategy's trades appear as the replay reaches them.

use super::Chart;
use crate::icons::ph;
use crate::motion;
use crate::theme::{self, Theme};
use egui::{Align2, Color32, CornerRadius, Id, Pos2, Rect, Sense, Stroke, Ui, pos2, vec2};
use opentern_core::Bar;
use std::time::{Duration, Instant};

/// Bars per second for each speed choice.
pub const SPEEDS: [(f32, &str); 6] = [
    (0.1, "10s"),
    (0.33, "3s"),
    (1.0, "1s"),
    (3.0, "0.3s"),
    (10.0, "0.1s"),
    (30.0, "0.03s"),
];

pub struct Replay {
    /// Choosing the bar to start from (the cursor shows where it cuts).
    pub selecting: bool,
    /// Every real bar, including the ones not replayed yet.
    pub full: Vec<Bar>,
    /// How many of `full` are on the chart.
    pub cut: usize,
    pub playing: bool,
    /// Index into `SPEEDS`.
    pub speed: usize,
    last_step: Option<Instant>,
}

impl Replay {
    fn selecting() -> Replay {
        Replay {
            selecting: true,
            full: Vec::new(),
            cut: 0,
            playing: false,
            speed: 2,
            last_step: None,
        }
    }

    pub fn left(&self) -> usize {
        self.full.len().saturating_sub(self.cut)
    }
}

impl Chart {
    pub fn replaying(&self) -> bool {
        self.replay.as_ref().is_some_and(|r| !r.selecting)
    }

    /// The Replay button: start choosing a bar (or stop choosing).
    pub fn toggle_replay(&mut self) {
        match &self.replay {
            None => self.replay = Some(Replay::selecting()),
            Some(r) if r.selecting && r.full.is_empty() => self.replay = None,
            Some(_) => self.exit_replay(),
        }
    }

    /// Starts the replay at shown bar `i`: it and everything before it stay.
    pub fn replay_from(&mut self, i: usize) {
        let Some(at) = self.bars.get(i).map(|b| b.time) else {
            return;
        };
        let (speed, full) = match self.replay.take() {
            Some(r) if !r.full.is_empty() => (r.speed, r.full),
            Some(r) => (r.speed, self.raw.clone()),
            None => (2, self.raw.clone()),
        };
        let cut = full.partition_point(|b| b.time <= at).max(1);
        self.raw = full[..cut].to_vec();
        self.replay = Some(Replay {
            selecting: false,
            full,
            cut,
            playing: false,
            speed,
            last_step: None,
        });
        self.rebuild();
        self.view.right = self.last_index() + super::RIGHT_MARGIN;
        self.data_version += 1;
    }

    /// One bar forward. False when there are none left.
    pub fn replay_step(&mut self) -> bool {
        let Some(r) = self.replay.as_mut() else { return false };
        if r.selecting || r.cut >= r.full.len() {
            r.playing = false;
            return false;
        }
        let b = r.full[r.cut];
        r.cut += 1;
        self.data_version += 1;
        self.tick(b);
        true
    }

    /// Back to live: every bar again.
    pub fn exit_replay(&mut self) {
        if let Some(r) = self.replay.take()
            && !r.full.is_empty()
        {
            self.raw = r.full;
            self.rebuild();
            self.view.right = self.last_index() + super::RIGHT_MARGIN;
        }
    }

    /// New data while replaying goes to the hidden bars; the chart keeps
    /// showing the replay. Returns true when it took the event.
    pub(super) fn replay_absorb(&mut self, e: &opentern_data::Event) -> bool {
        let Some(r) = self.replay.as_mut() else { return false };
        if r.selecting && r.full.is_empty() {
            return false;
        }
        match e {
            opentern_data::Event::Bars { bars, complete, .. } => {
                let shown = r.full.get(r.cut.saturating_sub(1)).map(|b| b.time);
                r.full = bars.clone();
                r.cut = shown.map(|t| r.full.partition_point(|b| b.time <= t)).unwrap_or(0);
                self.complete = *complete;
                true
            }
            opentern_data::Event::Tick { bar, .. } => {
                match r.full.last_mut() {
                    Some(last) if last.time == bar.time => *last = *bar,
                    Some(last) if last.time > bar.time => {}
                    _ => r.full.push(*bar),
                }
                true
            }
            _ => false,
        }
    }

    /// Steps while playing; asks for a repaint when the next step is due.
    pub fn replay_tick(&mut self, ctx: &egui::Context) {
        let Some(r) = self.replay.as_mut() else { return };
        if !r.playing {
            return;
        }
        let every = Duration::from_secs_f32(1.0 / SPEEDS[r.speed.min(SPEEDS.len() - 1)].0);
        let due = r.last_step.is_none_or(|t| t.elapsed() >= every);
        if due {
            r.last_step = Some(Instant::now());
            if !self.replay_step() {
                return;
            }
        }
        let wait = self
            .replay
            .as_ref()
            .and_then(|r| r.last_step)
            .map(|t| every.saturating_sub(t.elapsed()))
            .unwrap_or(every);
        ctx.request_repaint_after(wait);
    }
}

/// The replay's floating controls at the bottom of the main pane.
pub fn controls(ui: &mut Ui, chart: &mut Chart, t: &Theme, pane: Rect) {
    let Some(r) = chart.replay.as_ref() else { return };
    let selecting = r.selecting;
    let playing = r.playing;
    let speed = r.speed;
    let has_full = !r.full.is_empty();
    let left = r.left();
    // Wide enough for what's in it: the buttons, the bars-left count and ×.
    let speed_label = SPEEDS[speed.min(SPEEDS.len() - 1)].1;
    let measure = |text: String, font: egui::FontId| ui.painter().layout_no_wrap(text, font, t.text).size().x;
    let btn = |glyph: &str, label: &str| {
        let text = if label.is_empty() {
            glyph.to_string()
        } else {
            format!("{glyph}  {label}")
        };
        measure(text, theme::medium(12.5)) + 18.0
    };
    let w = if selecting && !has_full {
        8.0 + btn(ph::SCISSORS, "Select bar")
            + 8.0
            + measure("Click a bar to start from it".into(), theme::regular(12.5))
            + 44.0
    } else {
        8.0 + btn(ph::SCISSORS, "Select bar")
            + btn(ph::PAUSE, "")
            + btn(ph::SKIP_FORWARD, "")
            + btn(ph::GAUGE, speed_label)
            + btn(ph::FAST_FORWARD, "Real time")
            + 6.0
            + measure(format!("{left} left"), theme::regular(11.5))
            + 44.0
    };
    let bar = Rect::from_center_size(pos2(pane.center().x, pane.bottom() - 30.0), vec2(w, 36.0));
    let p = ui.painter();
    p.rect_filled(
        bar.translate(vec2(0.0, 2.0)),
        CornerRadius::same(10),
        Color32::from_black_alpha(40),
    );
    p.rect_filled(bar, CornerRadius::same(10), t.elevated);
    p.rect_stroke(
        bar,
        CornerRadius::same(10),
        Stroke::new(1.0, t.border),
        egui::StrokeKind::Inside,
    );
    let mut x = bar.left() + 8.0;
    let mut button = |ui: &mut Ui, glyph: &str, label: &str, tip: &str, on: bool, key: &str| -> bool {
        let font = theme::medium(12.5);
        let text = if label.is_empty() {
            glyph.to_string()
        } else {
            format!("{glyph}  {label}")
        };
        let gw = ui.painter().layout_no_wrap(text.clone(), font.clone(), t.text).size().x;
        let r = Rect::from_min_size(pos2(x, bar.top() + 5.0), vec2(gw + 16.0, 26.0));
        x = r.right() + 2.0;
        let resp = ui.interact(r, Id::new(("replay", chart_key(bar.min), key)), Sense::click());
        let hov = motion::toggle(ui.ctx(), resp.id, resp.hovered(), motion::MICRO);
        let p = ui.painter();
        p.rect_filled(r, CornerRadius::same(6), motion::with_alpha(t.hover_wash(), hov));
        if on {
            p.rect_filled(r, CornerRadius::same(6), t.accent_soft());
        }
        p.text(
            r.center(),
            Align2::CENTER_CENTER,
            text,
            font,
            if on { t.accent } else { t.text },
        );
        resp.on_hover_text(tip)
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .clicked()
    };
    if button(
        ui,
        ph::SCISSORS,
        "Select bar",
        "Pick a new starting bar",
        selecting,
        "select",
    ) && let Some(r) = chart.replay.as_mut()
    {
        r.selecting = !r.selecting;
        r.playing = false;
    }
    if selecting && !has_full {
        ui.painter().text(
            pos2(x + 8.0, bar.center().y),
            Align2::LEFT_CENTER,
            "Click a bar to start from it",
            theme::regular(12.5),
            t.text2,
        );
        let close = Rect::from_center_size(pos2(bar.right() - 20.0, bar.center().y), vec2(26.0, 26.0));
        let resp = ui.interact(close, Id::new(("replay-close", chart_key(bar.min))), Sense::click());
        ui.painter().text(
            close.center(),
            Align2::CENTER_CENTER,
            ph::X,
            theme::regular(14.0),
            t.text2,
        );
        if resp.on_hover_text("Close replay").clicked() {
            chart.replay = None;
        }
        return;
    }
    let (glyph, tip) = if playing {
        (ph::PAUSE, "Pause")
    } else {
        (ph::PLAY, "Play")
    };
    if button(ui, glyph, "", tip, false, "play")
        && let Some(r) = chart.replay.as_mut()
    {
        r.playing = !r.playing && r.cut < r.full.len();
        r.selecting = false;
    }
    if button(ui, ph::SKIP_FORWARD, "", "Forward one bar", false, "step") {
        if let Some(r) = chart.replay.as_mut() {
            r.selecting = false;
        }
        chart.replay_step();
    }
    if button(
        ui,
        ph::GAUGE,
        speed_label,
        "Speed: time per bar (click to change)",
        false,
        "speed",
    ) && let Some(r) = chart.replay.as_mut()
    {
        r.speed = (r.speed + 1) % SPEEDS.len();
    }
    if button(ui, ph::FAST_FORWARD, "Real time", "Jump to real time", false, "live") {
        chart.exit_replay();
        return;
    }
    ui.painter().text(
        pos2(x + 6.0, bar.center().y),
        Align2::LEFT_CENTER,
        format!("{left} left"),
        theme::regular(11.5),
        t.text3,
    );
    let close = Rect::from_center_size(pos2(bar.right() - 20.0, bar.center().y), vec2(26.0, 26.0));
    let resp = ui.interact(close, Id::new(("replay-close", chart_key(bar.min))), Sense::click());
    ui.painter().text(
        close.center(),
        Align2::CENTER_CENTER,
        ph::X,
        theme::regular(14.0),
        t.text2,
    );
    if resp.on_hover_text("Close replay").clicked() {
        chart.exit_replay();
    }
}

fn chart_key(p: Pos2) -> (i32, i32) {
    (p.x as i32, p.y as i32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use opentern_core::Timeframe;
    use opentern_data::Event;

    fn bars(n: usize) -> Vec<Bar> {
        (0..n)
            .map(|i| {
                let c = 100.0 + (i as f64 * 0.3).sin() * 5.0;
                Bar {
                    time: i as i64 * 3_600_000,
                    open: c - 0.5,
                    high: c + 1.0,
                    low: c - 1.0,
                    close: c,
                    volume: 10.0,
                }
            })
            .collect()
    }

    #[test]
    fn replay_cuts_steps_and_ends_where_live_is() {
        let sym = opentern_core::starter_watchlist().remove(0);
        let mut c = Chart::new(1, sym, Timeframe::parse("1h").unwrap());
        let rsi = opentern_series::builtins::by_id("rsi").unwrap();
        c.add_study(rsi, &rsi.default_inputs(), super::super::StudyLook::defaults(rsi), None);
        c.apply(Event::Bars {
            chart: 1,
            bars: bars(200),
            complete: true,
        });
        let full_rsi = c.studies[0].inst.out.plots[0].clone();
        c.toggle_replay();
        c.replay_from(99);
        assert_eq!(c.bars.len(), 100);
        // A live tick goes to the hidden bars, not the chart.
        c.apply(Event::Tick {
            chart: 1,
            bar: bars(201)[200],
        });
        assert_eq!(c.bars.len(), 100);
        for _ in 0..50 {
            assert!(c.replay_step());
        }
        assert_eq!(c.bars.len(), 150);
        // Values after stepping equal the full run's (studies stepped bar by bar).
        let v = &c.studies[0].inst.out.plots[0];
        for i in 0..149 {
            let (a, b) = (v[i], full_rsi[i]);
            assert!((a.is_nan() && b.is_nan()) || (a - b).abs() < 1e-9, "bar {i}");
        }
        c.exit_replay();
        assert_eq!(c.bars.len(), 201);
        assert!(c.replay.is_none());
    }
}

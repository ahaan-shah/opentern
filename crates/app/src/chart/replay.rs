//! Bar replay, as on TradingView: pick a bar, the chart cuts back to it,
//! then steps forward one bar at a time (by hand or playing at a speed).
//! Studies and strategies see only the bars replayed so far, so a
//! strategy's trades appear as the replay reaches them.

use super::Chart;
use crate::i18n::{tr, trf};
use crate::icons::ph;
use crate::motion;
use crate::theme::{self, Theme};
use egui::{Align2, CornerRadius, Id, Pos2, Rect, Sense, Stroke, Ui, pos2, vec2};
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

fn bar_id() -> Id {
    Id::new("replay-bar-rect")
}

/// Where a replay's controls are on screen, when one was drawn this frame
/// or the last: their spot is fixed, so other floating bars move instead.
pub fn bar_rect(ctx: &egui::Context) -> Option<Rect> {
    let now = ctx.cumulative_frame_nr();
    ctx.data(|d| d.get_temp::<(Rect, u64)>(bar_id()))
        .filter(|(_, f)| *f + 1 >= now)
        .map(|(r, _)| r)
}

/// The replay's floating controls at the bottom of the main pane, in the
/// favourite drawings toolbar's look: the same height, corners, shadow,
/// square icon buttons (names in their tooltips) and dividers.
pub fn controls(ui: &mut Ui, chart: &mut Chart, t: &Theme, pane: Rect) {
    const BTN: f32 = 32.0;
    const GAP: f32 = 2.0;
    const PAD: f32 = 5.0;
    const RULE: f32 = 9.0;
    let Some(r) = chart.replay.as_ref() else { return };
    let selecting = r.selecting;
    let playing = r.playing;
    let speed = r.speed;
    let has_full = !r.full.is_empty();
    let left = r.left();
    let speed_label = SPEEDS[speed.min(SPEEDS.len() - 1)].1;
    let measure = |text: &str, font: egui::FontId| ui.painter().layout_no_wrap(text.to_string(), font, t.text).size().x;
    let speed_text = format!("{}  {speed_label}", ph::GAUGE);
    let speed_w = measure(&speed_text, theme::medium(12.5)) + 16.0;
    let hint = tr("Click a bar to start from it");
    let left_text = trf!("{left} left");
    let picking = selecting && !has_full;
    let w = if picking {
        PAD + BTN + RULE + measure(hint, theme::regular(12.5)) + 12.0 + BTN + PAD
    } else {
        PAD + BTN
            + RULE
            + (BTN + GAP) * 2.0
            + speed_w
            + RULE
            + BTN
            + 8.0
            + measure(&left_text, theme::regular(12.0))
            + 8.0
            + BTN
            + PAD
    };
    let h = BTN + PAD * 2.0;
    let bar = Rect::from_center_size(pos2(pane.center().x, pane.bottom() - h / 2.0 - 12.0), vec2(w, h));
    // The favourite drawings toolbar steps out of its way (`favbar.rs`).
    let frame = ui.ctx().cumulative_frame_nr();
    ui.ctx().data_mut(|d| d.insert_temp(bar_id(), (bar, frame)));
    let radius = CornerRadius::same(12);
    let p = ui.painter();
    p.add(ui.visuals().popup_shadow.as_shape(bar, radius));
    p.rect(
        bar,
        radius,
        t.popup(),
        Stroke::new(1.0, t.edge()),
        egui::StrokeKind::Inside,
    );
    let key = chart_key(bar.min);
    let mut x = bar.left() + PAD;
    let top = bar.top() + PAD;
    let rule = |ui: &Ui, x: &mut f32| {
        let cx = *x + RULE / 2.0 - GAP / 2.0;
        ui.painter()
            .vline(cx, bar.y_range().shrink(PAD + 6.0), Stroke::new(1.0, t.border));
        *x += RULE - GAP;
    };
    let icon = |ui: &mut Ui, x: &mut f32, glyph: &str, tip: &str, on: bool, id: &str| -> bool {
        let r = Rect::from_min_size(pos2(*x, top), vec2(BTN, BTN));
        *x = r.right() + GAP;
        let resp = ui.interact(r, Id::new(("replay", key, id)), Sense::click());
        crate::widgets::tool_button_look(ui, t, resp, glyph, tip, on).clicked()
    };
    if icon(ui, &mut x, ph::SCISSORS, tr("Select bar"), selecting, "select")
        && let Some(r) = chart.replay.as_mut()
    {
        r.selecting = !r.selecting;
        r.playing = false;
    }
    rule(ui, &mut x);
    if picking {
        ui.painter().text(
            pos2(x + 6.0, bar.center().y),
            Align2::LEFT_CENTER,
            hint,
            theme::regular(12.5),
            t.text2,
        );
        let mut xr = bar.right() - PAD - BTN;
        if icon(ui, &mut xr, ph::X, tr("Close"), false, "close") {
            chart.replay = None;
        }
        return;
    }
    let (glyph, tip) = if playing {
        (ph::PAUSE, tr("Pause"))
    } else {
        (ph::PLAY, tr("Play"))
    };
    if icon(ui, &mut x, glyph, tip, playing, "play")
        && let Some(r) = chart.replay.as_mut()
    {
        r.playing = !r.playing && r.cut < r.full.len();
        r.selecting = false;
    }
    if icon(ui, &mut x, ph::SKIP_FORWARD, tr("Forward one bar"), false, "step") {
        if let Some(r) = chart.replay.as_mut() {
            r.selecting = false;
        }
        chart.replay_step();
    }
    // The speed: its value in words, so a button with text.
    let sr = Rect::from_min_size(pos2(x, top), vec2(speed_w, BTN));
    x = sr.right() + GAP;
    let resp = ui.interact(sr, Id::new(("replay", key, "speed")), Sense::click());
    let hov = motion::toggle(ui.ctx(), resp.id, resp.hovered(), motion::MICRO);
    let pt = ui.painter();
    pt.rect_filled(sr, CornerRadius::same(7), motion::with_alpha(t.hover_wash(), hov));
    pt.text(
        sr.center(),
        Align2::CENTER_CENTER,
        &speed_text,
        theme::medium(12.5),
        motion::lerp_color(t.text2, t.text, hov),
    );
    if resp
        .on_hover_text(tr("Speed: time per bar (click to change)"))
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
        && let Some(r) = chart.replay.as_mut()
    {
        r.speed = (r.speed + 1) % SPEEDS.len();
    }
    rule(ui, &mut x);
    if icon(ui, &mut x, ph::FAST_FORWARD, tr("Jump to real time"), false, "live") {
        chart.exit_replay();
        return;
    }
    ui.painter().text(
        pos2(x + 6.0, bar.center().y),
        Align2::LEFT_CENTER,
        left_text,
        theme::regular(12.0),
        t.text3,
    );
    let mut xr = bar.right() - PAD - BTN;
    if icon(ui, &mut xr, ph::X, tr("Close"), false, "close") {
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

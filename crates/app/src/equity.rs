//! The Strategy Report's equity chart: equity, buy & hold and drawdown, in
//! money or percent, on a linear or log scale. Its legend turns each line on
//! and off; dragging across it zooms to that range (the wheel zooms too, a
//! double-click goes back), and a click shows that bar on the price chart.
//! The worst close-to-close drawdown's fall is shaded, and while zoomed a row gives
//! the range's P&L, drawdown and trades.
//!
//! Buy & hold is the whole capital at the first bar's open, as the report's
//! Buy & hold return; drawdown runs from the bar-close equity's peak, as the
//! Performance tab's Max equity drawdown.

use crate::i18n::{date, tr, trf};
use crate::shell::small_button;
use crate::strategy::{Fmt, grouped, pct, tone};
use crate::theme::{self, Theme};
use egui::{Align2, Color32, CornerRadius, Pos2, Rect, Sense, Stroke, Ui, pos2, vec2};
use opentern_core::Bar;
use opentern_engine::Broker;
use opentern_series::Rgba;

/// What the chart draws, worked out once per strategy run.
#[derive(Clone, Debug, Default)]
pub struct Curves {
    pub cap: f64,
    pub time: Vec<i64>,
    /// Equity less the initial capital.
    pub pnl: Vec<f64>,
    /// Buy & hold's profit: the capital at the first bar's open.
    pub hold: Vec<f64>,
    /// Equity under its running peak (zero or below), in money and as a
    /// percent of the peak (none while the peak isn't above zero).
    pub dd: Vec<f64>,
    pub dd_pct: Vec<Option<f64>>,
    /// The worst drawdown: its peak, its low and the bar it was made up
    /// (none while it hasn't been).
    pub worst: Option<Worst>,
    /// Closed trades by exit bar: (exit bar, index in `Broker::trades`).
    exits: Vec<(usize, usize)>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Worst {
    pub peak: usize,
    pub low: usize,
    pub recovered: Option<usize>,
    pub money: f64,
    pub pct: Option<f64>,
}

impl Curves {
    pub fn new(b: &Broker, bars: &[Bar]) -> Curves {
        let n = b.equity.len().min(bars.len());
        let cap = b.props.initial_capital;
        let equity = &b.equity[..n];
        let first_open = bars.first().map_or(0.0, |x| x.open);
        let hold = bars[..n]
            .iter()
            .map(|x| {
                if first_open > 0.0 {
                    cap * (x.close / first_open - 1.0)
                } else {
                    0.0
                }
            })
            .collect();
        let mut dd = Vec::with_capacity(n);
        let mut dd_pct = Vec::with_capacity(n);
        let mut peak = f64::MIN;
        let mut peak_at = 0;
        let mut worst: Option<Worst> = None;
        for (i, &e) in equity.iter().enumerate() {
            if e >= peak {
                // Made up: the worst one so far ends here if it began at
                // the peak just left.
                if let Some(w) = worst.as_mut()
                    && w.recovered.is_none()
                    && w.peak == peak_at
                    && i > w.low
                {
                    w.recovered = Some(i);
                }
                peak = e;
                peak_at = i;
            }
            let d = e - peak;
            let p = (peak > 0.0).then(|| d / peak * 100.0);
            if d < 0.0 && worst.is_none_or(|w| d < w.money) {
                worst = Some(Worst {
                    peak: peak_at,
                    low: i,
                    recovered: None,
                    money: d,
                    pct: p,
                });
            }
            dd.push(d);
            dd_pct.push(p);
        }
        let mut exits: Vec<(usize, usize)> = b
            .trades
            .iter()
            .enumerate()
            .filter(|(_, t)| t.exit_bar < n)
            .map(|(k, t)| (t.exit_bar, k))
            .collect();
        exits.sort_unstable();
        Curves {
            cap,
            time: bars[..n].iter().map(|x| x.time).collect(),
            pnl: equity.iter().map(|e| e - cap).collect(),
            hold,
            dd,
            dd_pct,
            worst,
            exits,
        }
    }

    pub fn len(&self) -> usize {
        self.pnl.len()
    }

    /// The trades that closed on bar `i`, as indices into `Broker::trades`.
    pub fn closed_on(&self, i: usize) -> impl Iterator<Item = usize> + '_ {
        let from = self.exits.partition_point(|e| e.0 < i);
        self.exits[from..].iter().take_while(move |e| e.0 == i).map(|e| e.1)
    }

    /// P&L, return, drawdown and trades between bars `a` and `b`.
    pub fn range(&self, broker: &Broker, a: usize, b: usize) -> RangeStats {
        let b = b.min(self.len().saturating_sub(1));
        let a = a.min(b);
        let eq = |i: usize| self.cap + self.pnl[i];
        let pnl = self.pnl[b] - self.pnl[a];
        let ret = (eq(a) > 0.0).then(|| (eq(b) / eq(a) - 1.0) * 100.0);
        let (mut peak, mut dd, mut dd_pct) = (f64::MIN, 0.0f64, None);
        for i in a..=b {
            let e = eq(i);
            peak = peak.max(e);
            if e - peak < dd {
                dd = e - peak;
                dd_pct = (peak > 0.0).then(|| dd / peak * 100.0);
            }
        }
        let from = self.exits.partition_point(|e| e.0 <= a);
        let to = self.exits.partition_point(|e| e.0 <= b);
        let closed = &self.exits[from..to];
        let won = closed.iter().filter(|e| broker.trades[e.1].pnl > 0.0).count();
        RangeStats {
            pnl,
            ret,
            dd,
            dd_pct,
            trades: closed.len(),
            won,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RangeStats {
    pub pnl: f64,
    pub ret: Option<f64>,
    pub dd: f64,
    pub dd_pct: Option<f64>,
    /// Trades closed after the first bar, up to the last.
    pub trades: usize,
    pub won: usize,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Scale {
    #[default]
    Money,
    Percent,
}

/// How the chart is looked at; kept while the app runs.
#[derive(Clone, Debug)]
pub struct View {
    pub scale: Scale,
    pub log: bool,
    /// Equity, buy & hold, drawdown.
    pub shown: [bool; 3],
    /// The bars in view, as fractional indices; none for all of them.
    pub range: Option<(f64, f64)>,
    /// The strategy the range belongs to.
    pub of: u64,
}

impl Default for View {
    fn default() -> Self {
        View {
            scale: Scale::Money,
            log: false,
            shown: [true; 3],
            range: None,
            of: 0,
        }
    }
}

/// The fewest bars a zoom shows.
const MIN_SPAN: f64 = 4.0;

impl View {
    /// The bars in view for `n` bars.
    pub fn span(&self, n: usize) -> (f64, f64) {
        let last = n.saturating_sub(1) as f64;
        match self.range {
            Some((a, b)) => (a.max(0.0), b.min(last)),
            None => (0.0, last),
        }
    }

    /// Shows bars `a` to `b`, kept inside the data; all of them once the
    /// range covers everything.
    pub fn zoom_to(&mut self, a: f64, b: f64, n: usize) {
        let last = n.saturating_sub(1) as f64;
        let (mut a, mut b) = if a <= b { (a, b) } else { (b, a) };
        if b - a < MIN_SPAN {
            let mid = (a + b) / 2.0;
            a = mid - MIN_SPAN / 2.0;
            b = mid + MIN_SPAN / 2.0;
        }
        if a < 0.0 {
            b -= a;
            a = 0.0;
        }
        if b > last {
            a -= b - last;
            b = last;
        }
        self.range = if a <= 0.0 && b >= last || last < MIN_SPAN {
            None
        } else {
            Some((a.max(0.0), b))
        };
    }

    /// Zooms by `factor` (above 1 closer) around bar `at`.
    pub fn zoom_at(&mut self, at: f64, factor: f64, n: usize) {
        let (a, b) = self.span(n);
        self.zoom_to(at - (at - a) / factor, at + (b - at) / factor, n);
    }

    /// Moves the view by `bars`.
    pub fn pan(&mut self, bars: f64, n: usize) {
        if let Some((a, b)) = self.range {
            let last = n.saturating_sub(1) as f64;
            let d = bars.clamp(-a, last - b);
            self.range = Some((a + d, b + d));
        }
    }
}

/// The settings key for the lines' colours (Strategy settings).
pub const COLORS_KEY: &str = "report_colors";

/// Saved colours: `[equity, buy & hold, drawdown]` as RRGGBBAA numbers,
/// `null` for the theme's.
pub fn parse_colors(json: &str) -> [Option<Rgba>; 3] {
    let v: Vec<Option<u32>> = serde_json::from_str(json).unwrap_or_default();
    std::array::from_fn(|i| v.get(i).copied().flatten().map(Rgba))
}

pub fn colors_json(c: &[Option<Rgba>; 3]) -> String {
    serde_json::to_string(&c.iter().map(|c| c.map(|c| c.0)).collect::<Vec<_>>()).unwrap_or_default()
}

/// The three lines' colours: the user's, else the theme's.
pub fn colors(t: &Theme, chosen: &[Option<Rgba>; 3]) -> [Color32; 3] {
    let default = [t.accent, t.text3, t.neg];
    std::array::from_fn(|i| chosen[i].map_or(default[i], crate::chart::style::c32))
}

/// Round steps (1, 2, 2.5, 5 × 10ⁿ) from `lo` to `hi`, about `count` of them.
pub fn ticks(lo: f64, hi: f64, count: usize) -> Vec<f64> {
    let span = hi - lo;
    if !span.is_finite() || span <= 0.0 || count == 0 {
        return Vec::new();
    }
    let raw = span / count as f64;
    let mag = 10f64.powf(raw.log10().floor());
    let step = [1.0, 2.0, 2.5, 5.0, 10.0]
        .into_iter()
        .map(|m| m * mag)
        .find(|s| *s >= raw)
        .unwrap_or(10.0 * mag);
    let first = (lo / step - 1e-9).ceil() as i64;
    let last = (hi / step + 1e-9).floor() as i64;
    (first..=last.min(first + 64)).map(|k| k as f64 * step).collect()
}

/// Values on the chart's own scale: money or percent, and the log scale's
/// base (the capital, or 100%), so a value can be placed by `ln(base + v)`.
struct Axis {
    lo: f64,
    hi: f64,
    log: Option<f64>,
    rect: Rect,
}

impl Axis {
    fn t(&self, v: f64) -> f64 {
        match self.log {
            Some(base) => (base + v).ln(),
            None => v,
        }
    }

    fn y(&self, v: f64) -> f32 {
        let (lo, hi) = (self.t(self.lo), self.t(self.hi));
        let k = if hi > lo { (self.t(v) - lo) / (hi - lo) } else { 0.5 };
        self.rect.bottom() - self.rect.height() * k as f32
    }
}

/// The chart's values for bar `i` on `scale`.
fn value(c: &Curves, series: usize, i: usize, scale: Scale) -> Option<f64> {
    let money = match series {
        0 => c.pnl[i],
        1 => c.hold[i],
        _ => {
            return match scale {
                Scale::Money => Some(c.dd[i]),
                Scale::Percent => c.dd_pct[i],
            };
        }
    };
    Some(match scale {
        Scale::Money => money,
        Scale::Percent => money / c.cap * 100.0,
    })
}

fn money_or_pct(scale: Scale, v: f64, f: &Fmt) -> String {
    match scale {
        Scale::Money => f.money(v),
        Scale::Percent => pct(v),
    }
}

fn signed(s: String, v: f64) -> String {
    if v > 0.0 { format!("+{s}") } else { s }
}

/// A short date for the time axis, finer for shorter views.
fn axis_date(ms: i64, span_ms: i64, f: &Fmt) -> String {
    let Ok(ts) = jiff::Timestamp::from_millisecond(ms) else {
        return String::new();
    };
    let z = ts.to_zoned(f.tz.clone());
    const DAY: i64 = 86_400_000;
    if span_ms > 400 * DAY {
        date(&z, "%b '%y")
    } else if span_ms > 3 * DAY {
        date(&z, "%d %b")
    } else {
        date(&z, "%a %H:%M")
    }
}

fn full_date(ms: i64, f: &Fmt) -> String {
    jiff::Timestamp::from_millisecond(ms)
        .map(|ts| date(&ts.to_zoned(f.tz.clone()), "%d %b %Y"))
        .unwrap_or_default()
}

const HEADER: f32 = 32.0;
const STATS: f32 = 22.0;
const TIME_AXIS: f32 = 20.0;

/// Draws the chart in `rect`. Returns the bar to show on the price chart
/// when one was clicked.
#[allow(clippy::too_many_arguments)]
pub fn show(
    ui: &mut Ui,
    t: &Theme,
    broker: &Broker,
    c: &Curves,
    view: &mut View,
    colors: [Color32; 3],
    f: &Fmt,
    rect: Rect,
) -> Option<usize> {
    let n = c.len();
    if n == 0 {
        return None;
    }
    let id = ui.make_persistent_id("equity-chart");
    let font = theme::regular(11.5);
    let (from, to) = view.span(n);
    let first = from.floor() as usize;
    let last = (to.ceil() as usize).min(n - 1);
    let percent_ok = c.cap > 0.0;
    if !percent_ok {
        view.scale = Scale::Money;
    }
    let scale = view.scale;
    let [eq_on, hold_on, dd_on] = view.shown;
    let curves_on = eq_on || hold_on;

    // The range of the lines in view, zero included, and whether a log
    // scale can show it (everything above zero equity).
    let mut lo = 0.0f64;
    let mut hi = 0.0f64;
    for s in [0, 1].into_iter().filter(|s| view.shown[*s]) {
        for i in first..=last {
            if let Some(v) = value(c, s, i, scale) {
                lo = lo.min(v);
                hi = hi.max(v);
            }
        }
    }
    let base = match scale {
        Scale::Money => c.cap,
        Scale::Percent => 100.0,
    };
    let log_ok = base > 0.0 && base + lo > 0.0;
    let log = (view.log && log_ok).then_some(base);

    // Header: the legend, then the scale's controls on the right.
    let header = Rect::from_min_size(rect.min, vec2(rect.width(), HEADER));
    let mut controls = ui.new_child(
        egui::UiBuilder::new()
            .id_salt(id.with("controls"))
            .max_rect(header)
            .layout(egui::Layout::right_to_left(egui::Align::Center)),
    );
    controls.spacing_mut().item_spacing.x = 4.0;
    let mut chosen = view.scale;
    controls.add_enabled_ui(percent_ok, |ui| {
        crate::widgets::segmented(
            ui,
            t,
            &mut chosen,
            &[(Scale::Money, tr("Regular")), (Scale::Percent, tr("Percent"))],
        );
    });
    view.scale = chosen;
    let log_resp = controls
        .add_enabled_ui(log_ok, |ui| small_button(ui, t, tr("Log"), view.log && log_ok))
        .inner;
    let log_resp = if log_ok {
        log_resp.on_hover_text(tr("Log scale"))
    } else {
        log_resp.on_disabled_hover_text(tr("Log scale needs equity above zero"))
    };
    if log_resp.clicked() {
        view.log = !view.log;
    }
    if view.range.is_some() {
        let r = crate::widgets::tool_button(
            &mut controls,
            t,
            crate::icons::ph::MAGNIFYING_GLASS_MINUS,
            tr("Show all (double-click)"),
            false,
            26.0,
        );
        if r.clicked() {
            view.range = None;
        }
    }
    let legend_end = controls.min_rect().left() - 8.0;
    let names = [tr("Equity"), tr("Buy & hold"), tr("Drawdown")];
    let p = ui.painter_at(rect);
    let mut x = rect.left() + 2.0;
    for (i, name) in names.iter().enumerate() {
        let g = p.layout_no_wrap(name.to_string(), theme::medium(12.0), t.text2);
        let w = g.size().x + 26.0;
        let hit = Rect::from_min_size(pos2(x, header.center().y - 12.0), vec2(w, 24.0));
        if hit.right() > legend_end {
            break;
        }
        let resp = ui
            .interact(hit, id.with(("legend", i)), Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        if resp.clicked() {
            view.shown[i] = !view.shown[i];
        }
        let on = view.shown[i];
        if resp.hovered() {
            p.rect_filled(hit, CornerRadius::same(6), t.hovered(t.card));
        }
        let mark = Rect::from_center_size(pos2(hit.left() + 11.0, hit.center().y), vec2(10.0, 10.0));
        if on {
            p.rect_filled(mark, CornerRadius::same(3), colors[i]);
        } else {
            p.rect_stroke(
                mark.shrink(0.5),
                CornerRadius::same(3),
                Stroke::new(1.0, t.text3),
                egui::StrokeKind::Inside,
            );
        }
        let text = if on { t.text2 } else { t.text3 };
        p.galley(pos2(hit.left() + 21.0, hit.center().y - g.size().y / 2.0), g, text);
        x += w + 4.0;
    }

    // The range's numbers, while zoomed.
    let mut top = header.bottom() + 4.0;
    if view.range.is_some() {
        let s = c.range(broker, first, last);
        let row = Rect::from_min_size(pos2(rect.left(), top), vec2(rect.width(), STATS));
        let span = format!("{} – {}", full_date(c.time[first], f), full_date(c.time[last], f));
        let pnl = match scale {
            Scale::Money => signed(f.money(s.pnl), s.pnl),
            Scale::Percent => s.ret.map_or_else(|| "—".into(), |r| signed(pct(r), r)),
        };
        let dd = match scale {
            Scale::Money => f.money(s.dd),
            Scale::Percent => s.dd_pct.map_or_else(|| "—".into(), pct),
        };
        let won = if s.trades > 0 {
            pct(s.won as f64 / s.trades as f64 * 100.0)
        } else {
            "—".into()
        };
        let parts: [(&str, String, Color32); 5] = [
            ("", span, t.text2),
            (tr("Net profit"), pnl, tone(t, s.pnl)),
            (tr("Max equity drawdown"), dd, if s.dd < 0.0 { t.neg } else { t.text }),
            (tr("Trades"), s.trades.to_string(), t.text),
            (tr("Profitable trades"), won, t.text),
        ];
        let mut x = row.left() + 2.0;
        for (label, value, color) in parts {
            let lg = (!label.is_empty()).then(|| p.layout_no_wrap(label.to_string(), font.clone(), t.text3));
            let vg = p.layout_no_wrap(value, theme::medium(11.5), color);
            let w = lg.as_ref().map_or(0.0, |g| g.size().x + 6.0) + vg.size().x;
            if x + w > row.right() {
                break;
            }
            let y = row.center().y;
            if let Some(lg) = lg {
                let lw = lg.size().x;
                p.galley(pos2(x, y - lg.size().y / 2.0), lg, t.text3);
                x += lw + 6.0;
            }
            let vw = vg.size().x;
            p.galley(pos2(x, y - vg.size().y / 2.0), vg, color);
            x += vw + 18.0;
        }
        top = row.bottom() + 2.0;
    }

    // Panes: the lines above, the drawdown below, the dates at the bottom.
    let labels: Vec<String> = ticks(lo, hi, 4)
        .into_iter()
        .chain([c.dd.iter().copied().fold(0.0, f64::min)])
        .map(|v| tick_label(scale, v))
        .collect();
    let gutter = crate::widgets::widest(ui, font.clone(), labels.iter().map(String::as_str)) + 16.0;
    let body = Rect::from_min_max(
        pos2(rect.left(), top + 4.0),
        pos2(rect.right() - gutter.max(48.0), rect.bottom() - TIME_AXIS),
    );
    if body.height() < 30.0 || body.width() < 60.0 {
        return None;
    }
    let dd_h = if curves_on {
        (body.height() * 0.24).clamp(30.0, 90.0)
    } else {
        body.height()
    };
    let main = Rect::from_min_max(
        body.min,
        pos2(body.right(), body.bottom() - dd_h - if dd_on { 10.0 } else { 0.0 }),
    );
    let main = if dd_on { main } else { body };
    let dd_rect = Rect::from_min_max(pos2(body.left(), body.bottom() - dd_h), body.max);
    let xs = |i: f64| body.left() + body.width() * ((i - from) / (to - from).max(1e-9)) as f32;
    let at_x = |x: f32| from + ((x - body.left()) / body.width()) as f64 * (to - from);
    let clip = Rect::from_min_max(pos2(body.left(), rect.top()), pos2(body.right() + 1.0, rect.bottom()));
    let pc = p.with_clip_rect(clip);
    let grid = Stroke::new(1.0, crate::motion::with_alpha(t.border, 0.45));
    let zero = Stroke::new(1.0, crate::motion::with_alpha(t.text3, 0.55));

    let mut axis = None;
    if curves_on {
        let pad = ((hi - lo) * 0.06).max(if scale == Scale::Money { 1e-6 } else { 1e-4 });
        let a = Axis {
            lo: if log.is_some() {
                lo - (lo + base) * 0.03
            } else {
                lo - pad
            },
            hi: if log.is_some() {
                hi + (hi + base) * 0.03
            } else {
                hi + pad
            },
            log,
            rect: main,
        };
        // Grid and labels, none closer than 15 px.
        let mut last_y = f32::MAX;
        for v in ticks(a.lo, a.hi, ((main.height() / 26.0) as usize).clamp(3, 6)) {
            let y = a.y(v);
            if (last_y - y).abs() < 15.0 || y < main.top() || y > main.bottom() {
                continue;
            }
            last_y = y;
            if v != 0.0 {
                p.hline(main.x_range(), y, grid);
            }
            p.text(
                pos2(body.right() + 8.0, y),
                Align2::LEFT_CENTER,
                tick_label(scale, v),
                font.clone(),
                if v == 0.0 { t.text2 } else { t.text3 },
            );
        }
        // The worst drawdown's fall, from its peak to its low.
        if let Some(w) = c.worst {
            let band = Rect::from_x_y_ranges(xs(w.peak as f64)..=xs(w.low as f64), main.y_range());
            pc.rect_filled(band, CornerRadius::ZERO, crate::motion::with_alpha(colors[2], 0.06));
        }
        let base_y = a.y(0.0);
        pc.hline(main.x_range(), base_y, zero);
        if hold_on {
            let pts: Vec<Pos2> = (first..=last)
                .filter_map(|i| Some(pos2(xs(i as f64), a.y(value(c, 1, i, scale)?))))
                .collect();
            line(&pc, pts, Stroke::new(1.2, colors[1]));
        }
        if eq_on {
            let pts: Vec<Pos2> = (first..=last)
                .filter_map(|i| Some(pos2(xs(i as f64), a.y(value(c, 0, i, scale)?))))
                .collect();
            pc.add(area(&pts, base_y, colors[0], t));
            line(&pc, pts, Stroke::new(1.8, colors[0]));
        }
        axis = Some(a);
    }
    let mut dd_axis = None;
    if dd_on {
        let low = (first..=last).filter_map(|i| value(c, 2, i, scale)).fold(0.0, f64::min);
        let a = Axis {
            lo: if low < 0.0 { low * 1.08 } else { -1.0 },
            hi: 0.0,
            log: None,
            rect: Rect::from_min_max(pos2(dd_rect.left(), dd_rect.top() + 2.0), dd_rect.max),
        };
        let top_y = a.y(0.0);
        pc.hline(dd_rect.x_range(), top_y, zero);
        p.text(
            pos2(body.right() + 8.0, top_y),
            Align2::LEFT_CENTER,
            tick_label(scale, 0.0),
            font.clone(),
            t.text3,
        );
        if low < 0.0 {
            // Runs of bars with a value, each filled down from zero.
            let mut run: Vec<Pos2> = Vec::new();
            let flush = |run: &mut Vec<Pos2>| {
                if run.len() > 1 {
                    pc.add(fill_to(run, top_y, crate::motion::with_alpha(colors[2], 0.16)));
                    line(&pc, std::mem::take(run), Stroke::new(1.0, colors[2]));
                }
                run.clear();
            };
            for i in first..=last {
                match value(c, 2, i, scale) {
                    Some(v) => run.push(pos2(xs(i as f64), a.y(v))),
                    None => flush(&mut run),
                }
            }
            flush(&mut run);
            p.text(
                pos2(body.right() + 8.0, a.y(low)),
                Align2::LEFT_CENTER,
                tick_label(scale, low),
                font.clone(),
                colors[2],
            );
        }
        dd_axis = Some(a);
    }

    // Dates along the bottom, about one per 120 px.
    let span_ms = c.time[last] - c.time[first];
    let count = ((body.width() / 120.0) as usize).max(2);
    let mut last_right = f32::MIN;
    for k in 0..=count {
        let i = (from + (to - from) * k as f64 / count as f64).round() as usize;
        let text = axis_date(c.time[i.min(n - 1)], span_ms, f);
        let g = p.layout_no_wrap(text, font.clone(), t.text3);
        let x = xs(i as f64);
        let left = (x - g.size().x / 2.0).clamp(body.left(), body.right() - g.size().x);
        if left < last_right + 12.0 {
            continue;
        }
        last_right = left + g.size().x;
        p.galley(pos2(left, body.bottom() + 5.0), g, t.text3);
    }

    // Hover, drag to zoom, wheel, click and double-click.
    let resp = ui
        .interact(body, id.with("plot"), Sense::click_and_drag())
        .on_hover_cursor(egui::CursorIcon::Crosshair);
    let mut jump = None;
    let index_at = |x: f32| at_x(x).round().clamp(0.0, (n - 1) as f64) as usize;
    if resp.double_clicked() {
        view.range = None;
    } else if resp.clicked()
        && let Some(pos) = resp.interact_pointer_pos()
    {
        jump = Some(index_at(pos.x));
    }
    let drag_id = id.with("drag-from");
    if resp.drag_started()
        && let Some(o) = ui.input(|i| i.pointer.press_origin())
    {
        ui.data_mut(|d| d.insert_temp(drag_id, at_x(o.x)));
    }
    let dragging_from: Option<f64> = ui.data(|d| d.get_temp(drag_id));
    if let (Some(a), Some(pos)) = (dragging_from, resp.interact_pointer_pos())
        && resp.dragged()
    {
        let b = at_x(pos.x.clamp(body.left(), body.right()));
        let band = Rect::from_x_y_ranges(xs(a.min(b))..=xs(a.max(b)), body.y_range());
        pc.rect_filled(band, CornerRadius::ZERO, crate::motion::with_alpha(t.accent, 0.10));
        pc.vline(
            xs(a),
            body.y_range(),
            Stroke::new(1.0, crate::motion::with_alpha(t.accent, 0.6)),
        );
        pc.vline(
            xs(b),
            body.y_range(),
            Stroke::new(1.0, crate::motion::with_alpha(t.accent, 0.6)),
        );
    }
    if resp.drag_stopped() {
        if let (Some(a), Some(pos)) = (dragging_from, resp.interact_pointer_pos()) {
            let b = at_x(pos.x.clamp(body.left(), body.right()));
            if (xs(a) - xs(b)).abs() >= 6.0 {
                view.zoom_to(a, b, n);
            }
        }
        ui.data_mut(|d| d.remove::<f64>(drag_id));
    }
    if resp.hovered()
        && let Some(pos) = resp.hover_pos()
    {
        let (scroll, zoom) = ui.input(|i| (i.smooth_scroll_delta, i.zoom_delta()));
        let at = at_x(pos.x);
        let factor = zoom as f64 * (scroll.y as f64 * 0.003).exp();
        if (factor - 1.0).abs() > 1e-4 {
            view.zoom_at(at, factor, n);
        }
        if scroll.x != 0.0 {
            view.pan(-(scroll.x as f64) / body.width() as f64 * (to - from), n);
        }
        if scroll != egui::Vec2::ZERO || zoom != 1.0 {
            ui.input_mut(|i| i.smooth_scroll_delta = egui::Vec2::ZERO);
        }
    }

    let hover = resp.hover_pos().filter(|_| !resp.dragged()).map(|pos| index_at(pos.x));
    if let Some(i) = hover {
        let x = xs(i as f64);
        pc.vline(
            x,
            body.y_range(),
            Stroke::new(1.0, crate::motion::with_alpha(t.text3, 0.5)),
        );
        for s in 0..3 {
            if !view.shown[s] {
                continue;
            }
            let a = if s == 2 { dd_axis.as_ref() } else { axis.as_ref() };
            if let (Some(a), Some(v)) = (a, value(c, s, i, scale)) {
                pc.circle_filled(pos2(x, a.y(v)), 3.0, colors[s]);
            }
        }
        tooltip(ui, t, broker, c, view, colors, f, i, x, body);
    }
    jump
}

fn tick_label(scale: Scale, v: f64) -> String {
    match scale {
        Scale::Money => {
            let a = v.abs();
            let dec = if a == 0.0 || a >= 100.0 {
                0
            } else if a >= 1.0 {
                1
            } else {
                2
            };
            grouped(v, dec)
        }
        Scale::Percent => {
            let a = v.abs();
            let dec = if a == 0.0 || a >= 10.0 { 0 } else { 1 };
            format!("{}%", grouped(v, dec))
        }
    }
}

fn line(p: &egui::Painter, pts: Vec<Pos2>, stroke: Stroke) {
    match pts.len() {
        0 => {}
        1 => {
            p.circle_filled(pts[0], stroke.width + 0.5, stroke.color);
        }
        _ => {
            p.add(egui::Shape::line(pts, stroke));
        }
    }
}

/// The equity's area: its own colour above zero, the loss colour below.
fn area(pts: &[Pos2], base: f32, color: Color32, t: &Theme) -> egui::Mesh {
    let up = crate::motion::with_alpha(color, 0.10);
    let down = crate::motion::with_alpha(t.neg, 0.10);
    let mut mesh = egui::Mesh::default();
    let mut quad = |a: Pos2, b: Pos2, col: Color32| {
        let k = mesh.vertices.len() as u32;
        mesh.colored_vertex(a, col);
        mesh.colored_vertex(b, col);
        mesh.colored_vertex(pos2(b.x, base), col);
        mesh.colored_vertex(pos2(a.x, base), col);
        mesh.add_triangle(k, k + 1, k + 2);
        mesh.add_triangle(k, k + 2, k + 3);
    };
    for w in pts.windows(2) {
        let (a, b) = (w[0], w[1]);
        let (above_a, above_b) = (a.y <= base, b.y <= base);
        if above_a == above_b {
            quad(a, b, if above_a { up } else { down });
        } else {
            let k = (base - a.y) / (b.y - a.y);
            let cross = pos2(a.x + (b.x - a.x) * k, base);
            quad(a, cross, if above_a { up } else { down });
            quad(cross, b, if above_b { up } else { down });
        }
    }
    mesh
}

fn fill_to(pts: &[Pos2], base: f32, col: Color32) -> egui::Mesh {
    let mut mesh = egui::Mesh::default();
    for w in pts.windows(2) {
        let k = mesh.vertices.len() as u32;
        mesh.colored_vertex(w[0], col);
        mesh.colored_vertex(w[1], col);
        mesh.colored_vertex(pos2(w[1].x, base), col);
        mesh.colored_vertex(pos2(w[0].x, base), col);
        mesh.add_triangle(k, k + 1, k + 2);
        mesh.add_triangle(k, k + 2, k + 3);
    }
    mesh
}

/// The read-out for bar `i`: its date, the lines shown, the worst drawdown
/// when inside it, and the trades closed on it.
#[allow(clippy::too_many_arguments)]
fn tooltip(
    ui: &Ui,
    t: &Theme,
    broker: &Broker,
    c: &Curves,
    view: &View,
    colors: [Color32; 3],
    f: &Fmt,
    i: usize,
    x: f32,
    body: Rect,
) {
    let scale = view.scale;
    let names = [tr("Equity"), tr("Buy & hold"), tr("Drawdown")];
    let mut rows: Vec<(Option<Color32>, String, String, Color32)> = Vec::new();
    for s in 0..3 {
        if !view.shown[s] {
            continue;
        }
        let v = value(c, s, i, scale);
        let text = v.map_or_else(|| "—".into(), |v| money_or_pct(scale, v, f));
        let col = v.map_or(t.text, |v| if s == 2 { t.text } else { tone(t, v) });
        rows.push((Some(colors[s]), names[s].to_string(), text, col));
    }
    if let Some(w) = c.worst
        && i >= w.peak
        && i <= w.recovered.unwrap_or(c.len() - 1)
    {
        let v = match scale {
            Scale::Money => f.money(w.money),
            Scale::Percent => w.pct.map_or_else(|| "—".into(), pct),
        };
        rows.push((None, tr("Max equity drawdown").to_string(), v, t.neg));
    }
    let closed: Vec<usize> = c.closed_on(i).collect();
    for &k in closed.iter().take(3) {
        let tr_ = &broker.trades[k];
        let side = if tr_.long { tr("Long") } else { tr("Short") };
        let v = match scale {
            Scale::Money => signed(f.money(tr_.pnl), tr_.pnl),
            Scale::Percent => signed(pct(tr_.pnl_pct), tr_.pnl_pct),
        };
        rows.push((None, trf!("Trade {} · {}", k + 1, side), v, tone(t, tr_.pnl)));
    }
    if closed.len() > 3 {
        let more = closed.len() - 3;
        let text = if more == 1 {
            tr("1 more trade").to_string()
        } else {
            trf!("{} more trades", more)
        };
        rows.push((None, text, String::new(), t.text3));
    }
    let p = ui.painter();
    let label_font = theme::regular(11.5);
    let value_font = theme::medium(11.5);
    let head = p.layout_no_wrap(
        crate::strategy::time_text(c.time[i], &f.tz),
        theme::medium(11.5),
        t.text2,
    );
    let laid: Vec<_> = rows
        .into_iter()
        .map(|(dot, l, v, col)| {
            (
                dot,
                p.layout_no_wrap(l, label_font.clone(), t.text3),
                p.layout_no_wrap(v, value_font.clone(), col),
                col,
            )
        })
        .collect();
    let label_w = laid
        .iter()
        .map(|r| r.1.size().x + if r.0.is_some() { 12.0 } else { 0.0 })
        .fold(0.0, f32::max);
    let value_w = laid.iter().map(|r| r.2.size().x).fold(0.0, f32::max);
    let row_h = 17.0;
    let w = (label_w + 16.0 + value_w).max(head.size().x) + 20.0;
    let h = 10.0 + head.size().y + 4.0 + row_h * laid.len() as f32 + 6.0;
    let mut at = pos2(x + 14.0, body.top() + 8.0);
    if at.x + w > body.right() {
        at.x = x - 14.0 - w;
    }
    at.x = at.x.max(body.left());
    at.y = at.y.min((body.bottom() - h).max(body.top()));
    let r = Rect::from_min_size(at, vec2(w, h));
    let p = ui
        .painter()
        .clone()
        .with_layer_id(egui::LayerId::new(egui::Order::Tooltip, ui.id().with("equity-tip")));
    p.rect_filled(
        r.translate(vec2(0.0, 2.0)),
        CornerRadius::same(8),
        Color32::from_black_alpha(if t.dark { 60 } else { 18 }),
    );
    p.rect(
        r,
        CornerRadius::same(8),
        t.popup(),
        Stroke::new(1.0, t.edge()),
        egui::StrokeKind::Inside,
    );
    let mut y = r.top() + 8.0;
    let head_h = head.size().y;
    p.galley(pos2(r.left() + 10.0, y), head, t.text2);
    y += head_h + 4.0;
    for (dot, lg, vg, col) in laid {
        let cy = y + row_h / 2.0;
        let mut lx = r.left() + 10.0;
        if let Some(d) = dot {
            p.rect_filled(
                Rect::from_center_size(pos2(lx + 4.0, cy), vec2(8.0, 8.0)),
                CornerRadius::same(2),
                d,
            );
            lx += 12.0;
        }
        p.galley(pos2(lx, cy - lg.size().y / 2.0), lg, t.text3);
        p.galley(pos2(r.right() - 10.0 - vg.size().x, cy - vg.size().y / 2.0), vg, col);
        y += row_h;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use opentern_engine::{Props, Trade};

    fn bar(i: i64, open: f64, close: f64) -> Bar {
        Bar {
            time: i * 86_400_000,
            open,
            high: open.max(close),
            low: open.min(close),
            close,
            volume: 1.0,
        }
    }

    fn broker(equity: &[f64]) -> Broker {
        let mut b = Broker::default();
        b.props = Props {
            initial_capital: 1000.0,
            ..Props::default()
        };
        b.equity = equity.to_vec();
        b
    }

    fn trade(exit_bar: usize, pnl: f64) -> Trade {
        Trade {
            entry_id: "L".into(),
            exit_id: "X".into(),
            long: true,
            qty: 1.0,
            entry_price: 1.0,
            entry_bar: 0,
            entry_time: 0,
            exit_price: 1.0,
            exit_bar,
            exit_time: 0,
            pnl,
            pnl_pct: pnl / 10.0,
            commission: 0.0,
            entry_commission: 0.0,
            runup: 0.0,
            drawdown: 0.0,
            entry_comment: String::new(),
            exit_comment: String::new(),
        }
    }

    #[test]
    fn curves_match_the_report() {
        let bars: Vec<Bar> = [(10.0, 11.0), (11.0, 12.0), (12.0, 9.0), (9.0, 13.0), (13.0, 12.5)]
            .iter()
            .enumerate()
            .map(|(i, (o, c))| bar(i as i64, *o, *c))
            .collect();
        let b = broker(&[1000.0, 1100.0, 900.0, 1150.0, 1050.0]);
        let c = Curves::new(&b, &bars);
        assert_eq!(c.pnl, [0.0, 100.0, -100.0, 150.0, 50.0]);
        // Buy & hold from the first bar's open, as the report's figure.
        assert!((c.hold[4] - 1000.0 * (12.5 / 10.0 - 1.0)).abs() < 1e-9);
        assert_eq!(c.dd, [0.0, 0.0, -200.0, 0.0, -100.0]);
        assert_eq!(c.dd_pct[2], Some(-200.0 / 1100.0 * 100.0));
        // The worst drawdown: peak at bar 1, low at 2, made up at 3; the
        // Performance tab's Max equity drawdown.
        assert_eq!(
            c.worst.map(|w| (w.peak, w.low, w.recovered, w.money)),
            Some((1, 2, Some(3), -200.0))
        );
    }

    #[test]
    fn worst_drawdown_not_yet_made_up() {
        let bars: Vec<Bar> = (0..4).map(|i| bar(i, 10.0, 10.0)).collect();
        let c = Curves::new(&broker(&[1000.0, 1200.0, 1100.0, 900.0]), &bars);
        let w = c.worst.unwrap();
        assert_eq!((w.peak, w.low, w.recovered, w.money), (1, 3, None, -300.0));
    }

    #[test]
    fn no_trades_no_drawdown() {
        let bars: Vec<Bar> = (0..3).map(|i| bar(i, 10.0, 10.0)).collect();
        let c = Curves::new(&broker(&[1000.0; 3]), &bars);
        assert!(c.worst.is_none());
        assert_eq!(c.dd, [0.0; 3]);
    }

    #[test]
    fn range_stats() {
        let bars: Vec<Bar> = (0..6).map(|i| bar(i, 10.0, 10.0)).collect();
        let mut b = broker(&[1000.0, 1100.0, 1000.0, 1200.0, 1150.0, 1300.0]);
        b.trades = vec![trade(1, 100.0), trade(2, -100.0), trade(3, 200.0), trade(5, 150.0)];
        let c = Curves::new(&b, &bars);
        let s = c.range(&b, 1, 4);
        assert_eq!(s.pnl, 50.0);
        assert!((s.ret.unwrap() - (1150.0 / 1100.0 - 1.0) * 100.0).abs() < 1e-9);
        assert_eq!(s.dd, -100.0);
        // Trades closed after bar 1, up to bar 4.
        assert_eq!((s.trades, s.won), (2, 1));
        let all = c.range(&b, 0, 5);
        assert_eq!((all.trades, all.won), (4, 3));
        assert_eq!(c.closed_on(2).collect::<Vec<_>>(), [1]);
        assert_eq!(c.closed_on(4).count(), 0);
    }

    #[test]
    fn zooming_stays_inside_the_data() {
        let n = 101;
        let mut v = View::default();
        v.zoom_to(20.0, 40.0, n);
        assert_eq!(v.range, Some((20.0, 40.0)));
        v.pan(-50.0, n);
        assert_eq!(v.range, Some((0.0, 20.0)));
        v.pan(500.0, n);
        assert_eq!(v.range, Some((80.0, 100.0)));
        // Never closer than a few bars.
        v.zoom_to(50.0, 50.5, n);
        let (a, b) = v.range.unwrap();
        assert!(b - a >= MIN_SPAN - 1e-9);
        // Zooming out past the data shows all of it.
        v.zoom_at(50.0, 0.01, n);
        assert_eq!(v.range, None);
        assert_eq!(v.span(n), (0.0, 100.0));
        v.zoom_at(25.0, 2.0, n);
        assert_eq!(v.range, Some((12.5, 62.5)));
    }

    #[test]
    fn colours_round_trip() {
        let c = [Some(Rgba(0x11223344)), None, Some(Rgba(0xFF0000FF))];
        assert_eq!(parse_colors(&colors_json(&c)), c);
        assert_eq!(parse_colors(""), [None; 3]);
        assert_eq!(parse_colors("not json"), [None; 3]);
    }

    #[test]
    fn round_ticks() {
        assert_eq!(ticks(0.0, 100.0, 4), [0.0, 25.0, 50.0, 75.0, 100.0]);
        assert_eq!(ticks(-1198.0, 2054.0, 4), [-1000.0, 0.0, 1000.0, 2000.0]);
        assert_eq!(ticks(0.13, 0.47, 3), [0.2, 0.4]);
        assert!(ticks(5.0, 5.0, 4).is_empty());
        assert!(ticks(0.0, f64::NAN, 4).is_empty());
    }

    #[test]
    fn log_axis_places_values_by_balance() {
        let a = Axis {
            lo: -500.0,
            hi: 1000.0,
            log: Some(1000.0),
            rect: Rect::from_min_max(pos2(0.0, 0.0), pos2(100.0, 100.0)),
        };
        // 500 → 2000 is a doubling, as 1000 → 2000: halfway on a log scale.
        assert!((a.y(0.0) - 50.0).abs() < 1e-3);
        assert_eq!(a.y(-500.0), 100.0);
        assert_eq!(a.y(1000.0), 0.0);
    }

    /// Draws the chart once with `events`, returning what was clicked.
    fn frame(ctx: &egui::Context, b: &Broker, c: &Curves, view: &mut View, events: Vec<egui::Event>) -> Option<usize> {
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(800.0, 300.0))),
            events,
            ..Default::default()
        };
        let t = Theme::by_name("Tern Dark");
        let f = Fmt {
            ccy: "USD".into(),
            tz: jiff::tz::TimeZone::UTC,
        };
        let mut out = None;
        let mut full = ctx.run_ui(input, |ui| {
            let rect = Rect::from_min_size(Pos2::ZERO, vec2(800.0, 300.0));
            out = show(ui, &t, b, c, view, colors(&t, &[None; 3]), &f, rect);
        });
        full.textures_delta.clear();
        out
    }

    fn sample() -> (Broker, Curves) {
        let bars: Vec<Bar> = (0..200)
            .map(|i| {
                let v = 100.0 + (i as f64 * 0.1).sin() * 10.0;
                bar(i, v, v + 1.0)
            })
            .collect();
        let equity: Vec<f64> = (0..200).map(|i| 1000.0 + (i as f64 * 0.05).sin() * 300.0).collect();
        let mut b = broker(&equity);
        b.trades = vec![trade(50, 20.0), trade(50, -5.0), trade(120, 40.0)];
        let c = Curves::new(&b, &bars);
        (b, c)
    }

    #[test]
    fn every_combination_draws() {
        let (b, c) = sample();
        let ctx = egui::Context::default();
        theme::install_fonts(&ctx, theme::FONTS[0].name);
        for mask in 0..8u8 {
            for scale in [Scale::Money, Scale::Percent] {
                for log in [false, true] {
                    for range in [None, Some((30.0, 90.0))] {
                        let mut v = View {
                            scale,
                            log,
                            shown: [mask & 1 != 0, mask & 2 != 0, mask & 4 != 0],
                            range,
                            of: 0,
                        };
                        let hover = vec![egui::Event::PointerMoved(pos2(300.0, 150.0))];
                        frame(&ctx, &b, &c, &mut v, hover);
                        frame(&ctx, &b, &c, &mut v, vec![]);
                    }
                }
            }
        }
    }

    #[test]
    fn drag_zooms_click_jumps_double_click_resets() {
        let (b, c) = sample();
        let ctx = egui::Context::default();
        theme::install_fonts(&ctx, theme::FONTS[0].name);
        let mut v = View::default();
        frame(&ctx, &b, &c, &mut v, vec![]);
        let press = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        let (a, z) = (pos2(150.0, 150.0), pos2(400.0, 150.0));
        frame(&ctx, &b, &c, &mut v, vec![egui::Event::PointerMoved(a), press(a, true)]);
        for x in [200.0, 300.0, 400.0] {
            frame(&ctx, &b, &c, &mut v, vec![egui::Event::PointerMoved(pos2(x, 150.0))]);
        }
        frame(&ctx, &b, &c, &mut v, vec![press(z, false)]);
        let (from, to) = v.range.expect("zoomed");
        assert!(from > 20.0 && to < 120.0 && to - from > 40.0, "{from} {to}");

        // A click without a drag asks for that bar on the price chart.
        let m = pos2(300.0, 150.0);
        frame(&ctx, &b, &c, &mut v, vec![egui::Event::PointerMoved(m)]);
        frame(&ctx, &b, &c, &mut v, vec![press(m, true)]);
        let clicked = frame(&ctx, &b, &c, &mut v, vec![press(m, false)]);
        let i = clicked.expect("a bar");
        assert!(i as f64 >= from && i as f64 <= to);

        // A second click soon after is a double-click: everything again.
        frame(&ctx, &b, &c, &mut v, vec![press(m, true)]);
        frame(&ctx, &b, &c, &mut v, vec![press(m, false)]);
        assert_eq!(v.range, None);
    }
}

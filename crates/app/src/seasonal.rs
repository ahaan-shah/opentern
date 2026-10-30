//! The Seasonals tab: the active chart's monthly returns, a year per row
//! (newest first), each month against the month before, the year against
//! the last, and a row counting rises and falls per column. A two-handled
//! slider picks the years; its middle grip slides the whole window.
//! The chart view (TradingView's toggle, top left) draws a line per year:
//! the change since the last close of the year before, by day of the year,
//! with an average line and a hover readout.
//!
//! Monthly bars are fetched once per symbol (`Request::Fetch`, seqs from
//! `SEQ_BASE`), cache first like every one-off fetch; daily bars for the
//! chart only once it's shown.

use crate::app::App;
use crate::icons::ph;
use crate::motion::lerp_color;
use crate::theme::{self, Theme};
use egui::{Align2, Color32, CornerRadius, Rect, Sense, Stroke, Ui, pos2, vec2};
use opentern_core::{Bar, Timeframe};
use opentern_data::Request;
use opentern_series::profile::{MonthRow, SeasonYear, monthly_returns, seasonality};

/// Fetch seqs for this tab: `SEQ_BASE + n`.
pub const SEQ_BASE: u64 = 1 << 45;

/// Months asked for: 50 years, more than any provider has.
const MONTHS: usize = 600;
/// Days asked for the chart: 40 years.
const DAYS: usize = 366 * 40;

const NAMES: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum View {
    Chart,
    #[default]
    Table,
}

#[derive(Default)]
pub struct Seasonal {
    /// The symbol `rows` are for (or being fetched for).
    symbol: String,
    seq: u64,
    monthly_seq: u64,
    pub view: View,
    /// Every year's line for the chart, from daily bars (`None` until
    /// they're asked for and back).
    daily: Option<Result<Vec<SeasonYear>, String>>,
    daily_seq: u64,
    daily_for: String,
    pub rows: Vec<MonthRow>,
    error: Option<String>,
    loading: bool,
    /// The years shown, inclusive; `None` = the last `years` (6, as
    /// TradingView).
    pub range: Option<(i16, i16)>,
    /// How many years to start with (`OPENTERN_STILL=season-years:<n>`).
    pub years: Option<i16>,
}

impl App {
    /// Asks for the chart symbol's monthly bars when they aren't here.
    pub(crate) fn seasonal_want(&mut self) {
        let sym = self.chart.symbol.clone();
        if self.seasonal.symbol == sym.id {
            return;
        }
        let s = &mut self.seasonal;
        s.symbol = sym.id.clone();
        s.seq += 1;
        s.monthly_seq = s.seq;
        s.daily = None;
        s.daily_for.clear();
        s.rows.clear();
        s.error = None;
        s.loading = true;
        s.range = None;
        self.hub.send(Request::Fetch {
            seq: SEQ_BASE + self.seasonal.seq,
            symbol: sym,
            tf: Timeframe::parse("1M").expect("valid"),
            count: MONTHS,
        });
    }

    /// Asks for daily bars for the chart view, once per symbol.
    fn seasonal_want_daily(&mut self) {
        let sym = self.chart.symbol.clone();
        let s = &mut self.seasonal;
        if s.daily_for == sym.id {
            return;
        }
        s.daily_for = sym.id.clone();
        s.daily = None;
        s.seq += 1;
        s.daily_seq = s.seq;
        self.hub.send(Request::Fetch {
            seq: SEQ_BASE + s.seq,
            symbol: sym,
            tf: Timeframe::parse("1D").expect("valid"),
            count: DAYS,
        });
    }

    pub(crate) fn seasonal_fetched(&mut self, seq: u64, id: &str, result: Result<Vec<Bar>, String>) {
        let s = &mut self.seasonal;
        let tz = jiff::tz::TimeZone::get(&self.chart.symbol.timezone).unwrap_or(jiff::tz::TimeZone::UTC);
        if seq == SEQ_BASE + s.daily_seq && id == s.daily_for {
            s.daily = Some(match result {
                Ok(bars) if !bars.is_empty() => Ok(seasonality(&bars, &tz, usize::MAX)),
                Ok(_) => Err("No daily history for this symbol.".into()),
                Err(e) => Err(e),
            });
            return;
        }
        if seq != SEQ_BASE + s.monthly_seq || id != s.symbol {
            return;
        }
        s.loading = false;
        match result {
            Ok(bars) if !bars.is_empty() => s.rows = monthly_returns(&bars, &tz),
            Ok(_) => s.error = Some("No monthly history for this symbol.".into()),
            Err(e) => s.error = Some(e),
        }
    }

    /// The table as the agent gets it, for years `from..=to` (the tab's
    /// years when not given; given ones move the tab's slider too). `None`
    /// while the monthly bars load.
    pub(crate) fn seasonal_report(
        &mut self,
        from: Option<i16>,
        to: Option<i16>,
    ) -> Option<Result<serde_json::Value, String>> {
        use serde_json::json;
        self.seasonal_want();
        let s = &mut self.seasonal;
        if s.loading {
            return None;
        }
        if let Some(e) = &s.error {
            return Some(Err(e.clone()));
        }
        let (Some(newest), Some(oldest)) = (s.rows.first().map(|r| r.year), s.rows.last().map(|r| r.year)) else {
            return Some(Err("No monthly history for this symbol.".into()));
        };
        let span = s.years.unwrap_or(6).max(1) - 1;
        let shown = s.range.unwrap_or(((newest - span).max(oldest), newest));
        let range = match (from, to) {
            (None, None) => shown,
            (f, t) => {
                let r = (
                    f.unwrap_or(oldest).clamp(oldest, newest),
                    t.unwrap_or(newest).clamp(oldest, newest),
                );
                let r = (r.0.min(r.1), r.0.max(r.1));
                s.range = Some(r);
                r
            }
        };
        let round = |v: f64| {
            if v.is_finite() {
                json!((v * 100.0).round() / 100.0)
            } else {
                json!(null)
            }
        };
        let rows: Vec<&MonthRow> = s
            .rows
            .iter()
            .filter(|r| r.year >= range.0 && r.year <= range.1)
            .collect();
        let years: Vec<serde_json::Value> = rows
            .iter()
            .map(|r| {
                let mut m = serde_json::Map::new();
                m.insert("year".into(), json!(r.year));
                for (k, n) in NAMES.iter().enumerate() {
                    m.insert(n[..3].to_lowercase(), round(r.months[k]));
                }
                m.insert("whole_year".into(), round(r.year_pct));
                serde_json::Value::Object(m)
            })
            .collect();
        let columns: Vec<serde_json::Value> = (0..13)
            .map(|k| {
                let vals: Vec<f64> = rows
                    .iter()
                    .map(|r| if k == 12 { r.year_pct } else { r.months[k] })
                    .filter(|v| v.is_finite())
                    .collect();
                let avg = if vals.is_empty() {
                    f64::NAN
                } else {
                    vals.iter().sum::<f64>() / vals.len() as f64
                };
                json!({
                    "column": if k == 12 { "Year" } else { NAMES[k] },
                    "rises": vals.iter().filter(|v| **v > 0.0).count(),
                    "falls": vals.iter().filter(|v| **v < 0.0).count(),
                    "average": round(avg),
                })
            })
            .collect();
        Some(Ok(json!({
            "symbol": s.symbol,
            "years_available": [oldest, newest],
            "years_shown": [range.0, range.1],
            "returns_pct": years,
            "per_column": columns,
        })))
    }

    pub(crate) fn seasonal_tab(&mut self, ui: &mut Ui, t: &Theme) {
        self.seasonal_want();
        let s = &self.seasonal;
        if s.loading {
            ui.add_space(40.0);
            ui.vertical_centered(|ui| ui.spinner());
            return;
        }
        if let Some(e) = &s.error {
            crate::widgets::empty_state(ui, t, ph::CALENDAR_X, e);
            return;
        }
        let (Some(newest), Some(oldest)) = (s.rows.first().map(|r| r.year), s.rows.last().map(|r| r.year)) else {
            crate::widgets::empty_state(ui, t, ph::CALENDAR_BLANK, "No monthly history");
            return;
        };
        let span = s.years.unwrap_or(6).max(1) - 1;
        let mut range = s.range.unwrap_or(((newest - span).max(oldest), newest));
        range = (range.0.clamp(oldest, newest), range.1.clamp(oldest, newest));
        ui.horizontal(|ui| {
            crate::widgets::segmented(
                ui,
                t,
                &mut self.seasonal.view,
                &[(View::Chart, ph::CHART_LINE), (View::Table, ph::TABLE)],
            );
        });
        ui.add_space(6.0);
        if oldest < newest {
            year_slider(ui, t, oldest, newest, &mut range);
            ui.add_space(10.0);
        }
        self.seasonal.range = Some(range);
        if self.seasonal.view == View::Chart {
            self.seasonal_want_daily();
            match &self.seasonal.daily {
                None => {
                    ui.add_space(40.0);
                    ui.vertical_centered(|ui| ui.spinner());
                }
                Some(Err(e)) => crate::widgets::empty_state(ui, t, ph::CALENDAR_X, e),
                Some(Ok(all)) => {
                    let years: Vec<&SeasonYear> =
                        all.iter().filter(|y| y.year >= range.0 && y.year <= range.1).collect();
                    year_chart(ui, t, &years);
                }
            }
            return;
        }
        let rows: Vec<&MonthRow> = self
            .seasonal
            .rows
            .iter()
            .filter(|r| r.year >= range.0 && r.year <= range.1)
            .collect();
        table(ui, t, &rows);
    }
}

/// Paints `text` in a cell, right-aligned (left for the first column).
fn cell_text(p: &egui::Painter, r: Rect, text: &str, left: bool, font: egui::FontId, color: Color32) {
    if left {
        p.text(
            r.left_center() + vec2(12.0, 0.0),
            Align2::LEFT_CENTER,
            text,
            font,
            color,
        );
    } else {
        p.text(
            r.right_center() - vec2(10.0, 0.0),
            Align2::RIGHT_CENTER,
            text,
            font,
            color,
        );
    }
}

/// A return as shown in a cell, with `dp` decimals (none from four digits
/// up).
fn pct_text(v: f64, dp: usize) -> String {
    if !v.is_finite() {
        "—".into()
    } else if v.abs() < 0.5 * 10f64.powi(-(dp as i32)) {
        // A sliver of a loss rounds to 0%, not -0%.
        format!("{:.dp$}%", 0.0)
    } else if v.abs() >= 1000.0 {
        format!("{v:.0}%")
    } else {
        format!("{v:.dp$}%")
    }
}

/// A return's cell colour: a soft green or red, stronger for the year.
fn tint(t: &Theme, v: f64, strong: bool) -> Color32 {
    if !v.is_finite() {
        return Color32::TRANSPARENT;
    }
    let c = if v >= 0.0 { t.pos } else { t.neg };
    lerp_color(t.bg, c, if strong { 0.62 } else { 0.24 })
}

fn table(ui: &mut Ui, t: &Theme, rows: &[&MonthRow]) {
    // A little room on the right so the Year column doesn't touch the edge.
    let width = ui.available_width() - 4.0;
    let first = 84.0_f32.min(width * 0.1);
    // Columns never get narrower than their widest number. When two
    // decimals don't fit, fewer do; past that the table scrolls sideways.
    let widest = |dp: usize| {
        rows.iter()
            .flat_map(|r| r.months.iter().map(|v| (*v, false)).chain([(r.year_pct, true)]))
            .map(|(v, strong)| {
                let f = if strong {
                    theme::semibold(12.5)
                } else {
                    theme::regular(12.5)
                };
                ui.painter().layout_no_wrap(pct_text(v, dp), f, Color32::WHITE).size().x
            })
            .fold(0.0_f32, f32::max)
            + 20.0
    };
    let fit = (width - first) / 13.0;
    let (dp, need) = (0..=2)
        .rev()
        .map(|dp| (dp, widest(dp)))
        .find(|(_, w)| *w <= fit)
        .unwrap_or((0, widest(0)));
    let col = fit.max(52.0).max(need);
    let total = first + col * 13.0;
    let row_h = 30.0;
    let border = Stroke::new(1.0, t.border.gamma_multiply(0.7));
    let full = col >= 86.0;
    let x_of = |k: usize| first + col * k as f32;
    egui::ScrollArea::horizontal().id_salt("seasonal-h").show(ui, |ui| {
        ui.set_min_width(total);
        // Header.
        let (head, _) = ui.allocate_exact_size(vec2(total, row_h), Sense::hover());
        let p = ui.painter();
        let font = theme::regular(12.5);
        cell_text(
            p,
            Rect::from_min_size(head.min, vec2(first, row_h)),
            "Date",
            true,
            font.clone(),
            t.text3,
        );
        for (k, n) in NAMES.iter().enumerate() {
            let r = Rect::from_min_size(head.min + vec2(x_of(k), 0.0), vec2(col, row_h));
            let name = if full { n.to_string() } else { n[..3].to_string() };
            cell_text(p, r, &name, false, font.clone(), t.text3);
        }
        let yr = Rect::from_min_size(head.min + vec2(x_of(12), 0.0), vec2(col, row_h));
        cell_text(p, yr, "Year", false, theme::semibold(12.5), t.text2);
        p.hline(head.x_range(), head.bottom(), border);
        // The footer (average, rises and falls) keeps its place under the years.
        let body_h = (ui.available_height() - 2.0 * row_h - 2.0).max(row_h);
        egui::ScrollArea::vertical()
            .id_salt("seasonal-rows")
            .max_height(body_h)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                for r in rows {
                    let (rect, resp) = ui.allocate_exact_size(vec2(total, row_h), Sense::hover());
                    let p = ui.painter();
                    let row_font = theme::regular(12.5);
                    let year = r.year.to_string();
                    cell_text(
                        p,
                        Rect::from_min_size(rect.min, vec2(first, row_h)),
                        &year,
                        true,
                        row_font.clone(),
                        t.text2,
                    );
                    let values = r.months.iter().map(|v| (*v, false)).chain([(r.year_pct, true)]);
                    // With decimals dropped, the exact value shows on hover.
                    let mut exact = None;
                    for (k, (v, strong)) in values.enumerate() {
                        let c = Rect::from_min_size(rect.min + vec2(x_of(k), 0.0), vec2(col, row_h));
                        if dp < 2 && v.is_finite() && resp.hover_pos().is_some_and(|h| c.contains(h)) {
                            exact = Some(pct_text(v, 2));
                        }
                        p.rect_filled(c.shrink(0.5), CornerRadius::ZERO, tint(t, v, strong));
                        let text = pct_text(v, dp);
                        let f = if strong {
                            theme::semibold(12.5)
                        } else {
                            row_font.clone()
                        };
                        cell_text(p, c, &text, false, f, if v.is_finite() { t.text } else { t.text3 });
                    }
                    p.hline(rect.x_range(), rect.bottom(), border);
                    if let Some(e) = exact {
                        resp.clone().on_hover_text(e);
                    }
                    if resp.hovered() {
                        p.rect_stroke(
                            rect,
                            CornerRadius::ZERO,
                            Stroke::new(1.0, t.text3),
                            egui::StrokeKind::Inside,
                        );
                    }
                }
            });
        // The average return per column.
        let (avg_row, _) = ui.allocate_exact_size(vec2(total, row_h), Sense::hover());
        let p = ui.painter();
        cell_text(
            p,
            Rect::from_min_size(avg_row.min, vec2(first, row_h)),
            "Average",
            true,
            theme::semibold(12.5),
            t.text2,
        );
        #[allow(clippy::needless_range_loop)]
        for k in 0..13 {
            let vals: Vec<f64> = rows
                .iter()
                .map(|r| if k == 12 { r.year_pct } else { r.months[k] })
                .filter(|v| v.is_finite())
                .collect();
            let avg = if vals.is_empty() {
                f64::NAN
            } else {
                vals.iter().sum::<f64>() / vals.len() as f64
            };
            let c = Rect::from_min_size(avg_row.min + vec2(x_of(k), 0.0), vec2(col, row_h));
            let color = if !avg.is_finite() {
                t.text3
            } else if avg >= 0.0 {
                t.pos
            } else {
                t.neg
            };
            cell_text(p, c, &pct_text(avg, dp), false, theme::semibold(12.5), color);
        }
        p.hline(avg_row.x_range(), avg_row.bottom(), border);
        // Rises and falls, per column.
        let (foot, _) = ui.allocate_exact_size(vec2(total, row_h), Sense::hover());
        let p = ui.painter();
        cell_text(
            p,
            Rect::from_min_size(foot.min, vec2(first, row_h)),
            if full { "Rises and falls" } else { "Up / down" },
            true,
            theme::semibold(12.5),
            t.text2,
        );
        #[allow(clippy::needless_range_loop)]
        for k in 0..13 {
            let vals: Vec<f64> = rows
                .iter()
                .map(|r| if k == 12 { r.year_pct } else { r.months[k] })
                .filter(|v| v.is_finite())
                .collect();
            let up = vals.iter().filter(|v| **v > 0.0).count();
            let down = vals.iter().filter(|v| **v < 0.0).count();
            let c = Rect::from_min_size(foot.min + vec2(x_of(k), 0.0), vec2(col, row_h));
            let f = theme::semibold(12.0);
            let mut x = c.right() - 10.0;
            // Drawn triangles: the fonts have no ▲/▼.
            for (n, up, col) in [(down, false, t.neg), (up, true, t.pos)] {
                if n == 0 {
                    continue;
                }
                let g = p.layout_no_wrap(n.to_string(), f.clone(), col);
                x -= g.size().x;
                p.galley(pos2(x, c.center().y - g.size().y / 2.0), g, col);
                x -= 10.0;
                let (cx, cy) = (x + 4.0, c.center().y);
                let pts = if up {
                    vec![pos2(cx - 4.5, cy + 3.5), pos2(cx + 4.5, cy + 3.5), pos2(cx, cy - 4.0)]
                } else {
                    vec![pos2(cx - 4.5, cy - 3.5), pos2(cx + 4.5, cy - 3.5), pos2(cx, cy + 4.0)]
                };
                p.add(egui::Shape::convex_polygon(pts, col, Stroke::NONE));
                x -= 10.0;
            }
        }
        p.hline(foot.x_range(), foot.bottom(), border);
    });
}

/// Days on the chart's x axis (a leap year's).
const DAYS_X: usize = 366;

/// A year's value at day `d` (0-based), carried forward from its last bar;
/// `None` before its first bar or after its last.
fn value_at(y: &SeasonYear, f: f64) -> Option<f64> {
    let first = y.points.first()?;
    let last = y.points.last()?;
    if f < first.0 - 1e-9 || f > last.0 + 1e-9 {
        return None;
    }
    let k = y.points.partition_point(|p| p.0 <= f + 1e-9);
    Some(y.points[k.saturating_sub(1)].1)
}

/// The average line: at each day, the mean of the years that have one.
fn average_line(years: &[&SeasonYear]) -> Vec<(f64, f64)> {
    (0..DAYS_X)
        .filter_map(|d| {
            let f = d as f64 / (DAYS_X - 1) as f64;
            let vals: Vec<f64> = years.iter().filter_map(|y| value_at(y, f)).collect();
            (vals.len() >= 2).then(|| (f, vals.iter().sum::<f64>() / vals.len() as f64))
        })
        .collect()
}

fn hex(c: u32) -> Color32 {
    Color32::from_rgb((c >> 16) as u8, (c >> 8) as u8, c as u8)
}

/// "20 Aug" for a fraction of the year.
fn day_label(f: f64) -> String {
    let d = (f * (DAYS_X - 1) as f64).round() as i16;
    jiff::civil::date(2024, 1, 1)
        .checked_add(jiff::Span::new().days(d))
        .map(|x| x.strftime("%-d %b").to_string())
        .unwrap_or_default()
}

/// A line per year (January at the left), coloured newest first as the
/// Seasonality indicator does; the current year thicker with a dot at its
/// end; the average dashed; tags with each year's latest change on the
/// right; and a readout of every year at the pointer's day.
fn year_chart(ui: &mut Ui, t: &Theme, years: &[&SeasonYear]) {
    use crate::chart::profile::SEASON_COLORS;
    let rect = ui.available_rect_before_wrap();
    let resp = ui.allocate_rect(rect, Sense::hover());
    let p = ui.painter_at(rect);
    let n = years.len();
    if n == 0 {
        return;
    }
    let color = |k: usize| hex(SEASON_COLORS[(n - 1 - k) % SEASON_COLORS.len()]);
    let avg = average_line(years);
    let tag_font = theme::medium(11.5);
    let axis_font = theme::regular(11.0);
    let tag_w = 118.0;
    let axis_w = 64.0;
    let month_h = 20.0;
    let plot = Rect::from_min_max(
        rect.min + vec2(4.0, 8.0),
        pos2(rect.right() - tag_w - axis_w, rect.bottom() - month_h),
    );
    if plot.width() < 80.0 || plot.height() < 60.0 {
        return;
    }
    // The value range over the lines shown.
    let (mut lo, mut hi) = (0.0_f64, 0.0_f64);
    for y in years {
        for (_, v) in &y.points {
            lo = lo.min(*v);
            hi = hi.max(*v);
        }
    }
    let pad = ((hi - lo) * 0.06).max(1.0);
    let (lo, hi) = (lo - pad, hi + pad);
    let x_of = |f: f64| plot.left() + f as f32 * plot.width();
    let y_of = |v: f64| plot.bottom() - ((v - lo) / (hi - lo)) as f32 * plot.height();
    let grid = Stroke::new(1.0, t.border.gamma_multiply(0.8));

    // Quarters, dashed, and the months under the plot.
    for m in [3, 6, 9] {
        let f = jiff::civil::date(2024, m + 1, 1).day_of_year() as f64 / DAYS_X as f64;
        let x = x_of(f - 1.0 / DAYS_X as f64);
        p.extend(egui::Shape::dashed_line(
            &[pos2(x, plot.top()), pos2(x, plot.bottom())],
            grid,
            4.0,
            4.0,
        ));
    }
    for (m, name) in NAMES.iter().enumerate() {
        p.text(
            pos2(
                plot.left() + plot.width() * (m as f32 + 0.5) / 12.0,
                rect.bottom() - 4.0,
            ),
            Align2::CENTER_BOTTOM,
            &name[..3],
            axis_font.clone(),
            t.text3,
        );
    }
    // The percent scale, and zero.
    let (ticks, _) = crate::chart::scale::price_ticks(lo, hi, plot.height(), 56.0);
    for v in ticks {
        p.text(
            pos2(plot.right() + 10.0, y_of(v)),
            Align2::LEFT_CENTER,
            format!("{v:.2}%"),
            axis_font.clone(),
            t.text3,
        );
    }
    p.hline(plot.x_range(), y_of(0.0), Stroke::new(1.0, t.text3.gamma_multiply(0.7)));

    // The lines, oldest first so the newest sits on top.
    let this_year = jiff::Zoned::now().year();
    for (k, y) in years.iter().enumerate() {
        let pts: Vec<_> = y.points.iter().map(|(f, v)| pos2(x_of(*f), y_of(*v))).collect();
        let current = y.year == this_year;
        if pts.len() >= 2 {
            p.add(egui::Shape::line(
                pts.clone(),
                Stroke::new(if current { 2.5 } else { 1.25 }, color(k)),
            ));
        }
        if current && let Some(end) = pts.last() {
            p.circle_filled(*end, 4.0, color(k));
        }
    }
    if avg.len() >= 2 {
        let pts: Vec<_> = avg.iter().map(|(f, v)| pos2(x_of(*f), y_of(*v))).collect();
        p.extend(egui::Shape::dashed_line(&pts, Stroke::new(1.5, t.text), 6.0, 4.0));
    }

    // Tags: each year's latest change, pushed apart so none overlap.
    let tag_h = 20.0;
    let mut tags: Vec<(f32, String, String, Color32)> = years
        .iter()
        .enumerate()
        .filter_map(|(k, y)| {
            let v = y.points.last()?.1;
            Some((y_of(v), y.year.to_string(), crate::format::signed_pct(v), color(k)))
        })
        .collect();
    if let Some((_, v)) = avg.last() {
        tags.push((y_of(*v), "Avg".into(), crate::format::signed_pct(*v), t.text2));
    }
    tags.sort_by(|a, b| a.0.total_cmp(&b.0));
    let (top, bottom) = (rect.top() + tag_h / 2.0, plot.bottom() + month_h - tag_h / 2.0);
    for k in 0..tags.len() {
        let min = if k == 0 { top } else { tags[k - 1].0 + tag_h + 2.0 };
        tags[k].0 = tags[k].0.max(min);
    }
    for k in (0..tags.len()).rev() {
        let max = if k + 1 == tags.len() {
            bottom
        } else {
            tags[k + 1].0 - tag_h - 2.0
        };
        tags[k].0 = tags[k].0.min(max);
    }
    let left = rect.right() - tag_w;
    for (y, year, value, c) in &tags {
        let r = Rect::from_min_size(pos2(left, y - tag_h / 2.0), vec2(tag_w - 2.0, tag_h));
        p.rect_filled(r, egui::CornerRadius::same(3), *c);
        let split = r.left() + 44.0;
        p.vline(
            split,
            r.y_range().shrink(3.0),
            Stroke::new(1.0, Color32::from_black_alpha(90)),
        );
        p.text(
            pos2(r.left() + 7.0, r.center().y),
            Align2::LEFT_CENTER,
            year,
            tag_font.clone(),
            Color32::WHITE,
        );
        p.text(
            pos2(r.right() - 7.0, r.center().y),
            Align2::RIGHT_CENTER,
            value,
            tag_font.clone(),
            Color32::WHITE,
        );
    }

    // The readout: a line at the pointer's day, a dot on each year, and
    // every year's change that day.
    let Some(hover) = resp.hover_pos().filter(|h| plot.contains(*h)) else {
        return;
    };
    let f = ((hover.x - plot.left()) / plot.width()).clamp(0.0, 1.0) as f64;
    let f = (f * (DAYS_X - 1) as f64).round() / (DAYS_X - 1) as f64;
    let x = x_of(f);
    p.vline(x, plot.y_range(), Stroke::new(1.0, t.text3));
    let mut lines: Vec<(String, f64, Color32)> = Vec::new();
    for (k, y) in years.iter().enumerate() {
        if let Some(v) = value_at(y, f) {
            p.circle_filled(pos2(x, y_of(v)), 3.5, color(k));
            lines.push((y.year.to_string(), v, color(k)));
        }
    }
    if let Some(v) = avg.iter().find(|(a, _)| (a - f).abs() < 1e-9).map(|(_, v)| *v) {
        lines.push(("Avg".into(), v, t.text2));
    }
    if lines.is_empty() {
        return;
    }
    let row = 20.0;
    let font = theme::medium(12.0);
    let value_w = lines
        .iter()
        .map(|(_, v, _)| {
            p.layout_no_wrap(crate::format::signed_pct(*v), font.clone(), t.text)
                .size()
                .x
        })
        .fold(0.0_f32, f32::max);
    let size = vec2(68.0 + value_w + 12.0, row * lines.len() as f32 + 46.0);
    let mut at = pos2(x + 14.0, hover.y - size.y / 2.0);
    if at.x + size.x > plot.right() {
        at.x = x - 14.0 - size.x;
    }
    at.y = at.y.clamp(rect.top() + 2.0, rect.bottom() - size.y - 2.0);
    let card = Rect::from_min_size(at, size);
    p.rect(
        card,
        egui::CornerRadius::same(8),
        t.elevated,
        Stroke::new(1.0, t.border),
        egui::StrokeKind::Inside,
    );
    for (k, (label, v, c)) in lines.iter().enumerate() {
        let cy = card.top() + 12.0 + row * (k as f32 + 0.5);
        p.circle_filled(pos2(card.left() + 16.0, cy), 4.0, *c);
        p.text(
            pos2(card.left() + 26.0, cy),
            Align2::LEFT_CENTER,
            label,
            font.clone(),
            t.text,
        );
        p.text(
            pos2(card.right() - 12.0, cy),
            Align2::RIGHT_CENTER,
            crate::format::signed_pct(*v),
            font.clone(),
            t.text,
        );
    }
    p.text(
        pos2(card.center().x, card.bottom() - 11.0),
        Align2::CENTER_BOTTOM,
        day_label(f),
        theme::regular(11.5),
        t.text3,
    );
}

/// The years slider: labels above, a track with two handles and a grip in
/// between that drags the whole window.
fn year_slider(ui: &mut Ui, t: &Theme, lo: i16, hi: i16, range: &mut (i16, i16)) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 46.0), Sense::hover());
    let track = Rect::from_min_max(
        pos2(rect.left() + 12.0, rect.bottom() - 16.0),
        pos2(rect.right() - 12.0, rect.bottom() - 8.0),
    );
    let span = (hi - lo) as f32;
    let x_of = |y: i16| track.left() + track.width() * (y - lo) as f32 / span;
    let year_at = |x: f32| (lo as f32 + ((x - track.left()) / track.width()).clamp(0.0, 1.0) * span).round() as i16;
    let p = ui.painter();
    // Labels: the ends and a few between, as round steps.
    let font = theme::medium(11.5);
    let steps = ((track.width() / 110.0).floor() as i16).clamp(1, 6);
    let step = ((hi - lo) as f32 / steps as f32).ceil().max(1.0) as i16;
    let mut y = lo;
    while y < hi {
        if hi - y >= step / 2 || y == lo {
            p.text(
                pos2(x_of(y), rect.top() + 6.0),
                Align2::CENTER_TOP,
                y.to_string(),
                font.clone(),
                t.text3,
            );
        }
        y += step;
    }
    p.text(
        pos2(x_of(hi), rect.top() + 6.0),
        Align2::CENTER_TOP,
        hi.to_string(),
        font.clone(),
        t.text2,
    );
    p.rect_filled(track, CornerRadius::same(4), t.hover_wash());
    // Interaction: each handle, and the band between them.
    let id = ui.id().with("seasonal-slider");
    let (a, b) = (x_of(range.0), x_of(range.1));
    let ha = Rect::from_center_size(pos2(a, track.center().y), vec2(20.0, 20.0));
    let hb = Rect::from_center_size(pos2(b, track.center().y), vec2(20.0, 20.0));
    let band = Rect::from_min_max(pos2(a + 10.0, track.top() - 6.0), pos2(b - 10.0, track.bottom() + 6.0));
    let ra = ui.interact(ha, id.with("a"), Sense::drag());
    let rb = ui.interact(hb, id.with("b"), Sense::drag());
    let rm = ui.interact(band, id.with("m"), Sense::drag());
    let pointer = ui.input(|i| i.pointer.interact_pos());
    if let Some(px) = pointer.map(|p| p.x) {
        if ra.dragged() {
            range.0 = year_at(px).min(range.1);
        } else if rb.dragged() {
            range.1 = year_at(px).max(range.0);
        }
    }
    if rm.drag_started() {
        ui.memory_mut(|m| {
            m.data.insert_temp(
                id.with("from"),
                (range.0, rm.interact_pointer_pos().map_or(0.0, |p| p.x)),
            )
        });
    }
    if rm.dragged()
        && let (Some((start, x0)), Some(px)) = (
            ui.memory(|m| m.data.get_temp::<(i16, f32)>(id.with("from"))),
            pointer.map(|p| p.x),
        )
    {
        let width = range.1 - range.0;
        let moved = ((px - x0) / track.width() * span).round() as i16;
        let from = (start + moved).clamp(lo, hi - width);
        *range = (from, from + width);
    }
    for r in [&ra, &rb, &rm] {
        if r.hovered() || r.dragged() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
        }
    }
    // The chosen window, its grip and the handles.
    let (a, b) = (x_of(range.0), x_of(range.1));
    let p = ui.painter();
    let sel = Rect::from_min_max(pos2(a, track.top()), pos2(b, track.bottom()));
    p.rect_filled(sel, CornerRadius::same(4), t.text2);
    if b - a > 34.0 {
        p.text(
            sel.center(),
            Align2::CENTER_CENTER,
            ph::DOTS_SIX,
            theme::regular(11.0),
            t.bg,
        );
    }
    for (x, r) in [(a, &ra), (b, &rb)] {
        let hot = r.hovered() || r.dragged();
        let c = pos2(x, track.center().y);
        p.circle_filled(c, if hot { 10.0 } else { 9.0 }, t.bg);
        p.circle_stroke(c, if hot { 10.0 } else { 9.0 }, Stroke::new(2.0, t.text));
    }
    if ra.hovered() || ra.dragged() || rb.hovered() || rb.dragged() || rm.dragged() {
        ra.on_hover_text(format!("{} – {}", range.0, range.1));
    }
}

#[cfg(test)]
mod tests {
    use super::pct_text;

    #[test]
    fn cell_text_drops_decimals_when_asked_and_from_four_digits() {
        assert_eq!(pct_text(-15.304, 2), "-15.30%");
        assert_eq!(pct_text(-15.304, 1), "-15.3%");
        assert_eq!(pct_text(-15.304, 0), "-15%");
        assert_eq!(pct_text(11177.78, 2), "11178%");
        assert_eq!(pct_text(-0.004, 2), "0.00%");
        assert_eq!(pct_text(-0.3, 0), "0%");
        assert_eq!(pct_text(f64::NAN, 2), "—");
    }
}

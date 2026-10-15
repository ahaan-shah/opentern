//! Strategies on the chart: their broker runs (a Pine strategy's own broker,
//! or a Python strategy's orders replayed through one), the trades drawn on
//! the chart, and the Strategy Report tab with TradingView's layout and
//! metrics (`opentern_engine::Report`; formulas in `docs/metrics.md`).

use crate::app::App;
use crate::chart::{MarkFill, TradeMarks};
use crate::shell::small_button;
use crate::theme::{self, Theme};
use egui::{Align, Align2, Color32, CornerRadius, Layout, Rect, Sense, Stroke, Ui, pos2, vec2};
use opentern_core::Bar;
use opentern_engine::{Broker, Order, OrderKind, Props, Report, Trade};
use opentern_pine::PineStudy;
use std::sync::Arc;

pub struct StrategyRun {
    pub uid: u64,
    pub name: String,
    pub broker: Broker,
    pub report: Report,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ReportTab {
    #[default]
    Overview,
    Performance,
    Trades,
    Ratios,
    List,
    Properties,
}

impl ReportTab {
    const ALL: [ReportTab; 6] = [
        ReportTab::Overview,
        ReportTab::Performance,
        ReportTab::Trades,
        ReportTab::Ratios,
        ReportTab::List,
        ReportTab::Properties,
    ];

    /// A shorter name for narrow windows.
    fn short(self) -> &'static str {
        match self {
            ReportTab::Overview => "Overview",
            ReportTab::Performance => "Performance",
            ReportTab::Trades => "Trades",
            ReportTab::Ratios => "Ratios",
            ReportTab::List => "List",
            ReportTab::Properties => "Properties",
        }
    }

    fn title(self) -> &'static str {
        match self {
            ReportTab::Overview => "Overview",
            ReportTab::Performance => "Performance",
            ReportTab::Trades => "Trades analysis",
            ReportTab::Ratios => "Risk/performance ratios",
            ReportTab::List => "List of trades",
            ReportTab::Properties => "Properties",
        }
    }
}

#[derive(Default)]
pub struct Strategies {
    pub runs: Vec<StrategyRun>,
    pub(crate) key: (u64, u64, u64),
    pub selected: Option<u64>,
    pub tab: ReportTab,
    pub status: String,
}

impl Strategies {
    pub fn current(&self) -> Option<&StrategyRun> {
        self.runs
            .iter()
            .find(|r| Some(r.uid) == self.selected)
            .or_else(|| self.runs.first())
    }
}

/// The broker of a strategy study's state, whatever language it's in.
pub(crate) fn broker_of(state: &dyn std::any::Any) -> Option<&Broker> {
    match state.downcast_ref::<PineStudy>() {
        Some(p) => p.broker(),
        None => state
            .downcast_ref::<opentern_script::ScriptStudy>()
            .and_then(|s| s.broker()),
    }
}

/// A Python strategy's per-bar orders, filled bar by bar by the broker.
pub fn replay(props: Props, intents: &[serde_json::Value], bars: &[Bar]) -> Broker {
    let mut by_bar: Vec<Vec<&serde_json::Value>> = vec![Vec::new(); bars.len()];
    for it in intents {
        if let Some(i) = it.get("bar").and_then(|b| b.as_u64())
            && let Some(v) = by_bar.get_mut(i as usize)
        {
            v.push(it);
        }
    }
    let mut b = Broker::new(props);
    let num = |it: &serde_json::Value, k: &str| it.get(k).and_then(|x| x.as_f64());
    let text = |it: &serde_json::Value, k: &str| it.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
    for (i, bar) in bars.iter().enumerate() {
        b.open_bar(i, bar);
        for it in &by_bar[i] {
            let id = text(it, "id");
            let long = it.get("long").and_then(|x| x.as_bool()).unwrap_or(true);
            let kind = match it.get("type").and_then(|x| x.as_str()) {
                Some("entry") => OrderKind::Entry,
                Some("order") => OrderKind::Order,
                Some("exit") => OrderKind::Exit,
                Some("close") | Some("close_all") => OrderKind::Close,
                _ => continue,
            };
            if kind == OrderKind::Close && !b.lots.iter().any(|l| id.is_empty() || l.id == id) {
                continue;
            }
            let mut o = Order::market(&id, kind, long);
            o.qty = num(it, "qty").filter(|q| *q > 0.0);
            o.limit = num(it, "limit");
            o.stop = num(it, "stop");
            o.from_entry = text(it, "from_entry");
            o.qty_percent = num(it, "qty_percent").unwrap_or(100.0);
            o.profit_ticks = num(it, "profit");
            o.loss_ticks = num(it, "loss");
            o.comment = text(it, "comment");
            if o.comment.is_empty() {
                o.comment = id.clone();
            }
            b.submit(o);
        }
        b.close_bar(bar);
    }
    b
}

fn qty_text(q: f64) -> String {
    let s = format!("{:.6}", q.abs());
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// What the chart draws for a strategy run.
pub fn marks(b: &Broker) -> TradeMarks {
    let fills = b
        .fills
        .iter()
        .map(|f| {
            let name = if f.comment.is_empty() { &f.id } else { &f.comment };
            let sign = if f.qty > 0.0 { "+" } else { "-" };
            MarkFill {
                bar: f.bar,
                price: f.price,
                buy: f.qty > 0.0,
                label: format!("{name} {sign}{}", qty_text(f.qty)),
            }
        })
        .collect();
    let links = b
        .trades
        .iter()
        .map(|t| (t.entry_bar, t.entry_price, t.exit_bar, t.exit_price, t.pnl > 0.0))
        .collect();
    let pos = b.position_size();
    TradeMarks {
        fills,
        links,
        position: (pos != 0.0).then(|| (b.position_avg_price(), pos > 0.0)),
    }
}

impl App {
    /// Rebuilds the strategy runs when the data, the studies or a Python
    /// result changed (never every frame), and the chart's trade marks.
    pub(crate) fn refresh_strategies(&mut self) {
        let py: u64 = self.scripts.iter().map(|r| r.out_seq).sum();
        let key = (self.chart.data_version, self.chart.changed, py);
        if key == self.strategies.key {
            return;
        }
        self.strategies.key = key;
        let mut runs = Vec::new();
        for s in &self.chart.studies {
            let pine = s.inst.latest_state().and_then(broker_of).cloned();
            let broker = match pine {
                Some(b) => b,
                None => {
                    let Some(run) = self
                        .scripts
                        .iter()
                        .find(|r| r.uid == Some(s.uid) && r.kind == "strategy")
                    else {
                        continue;
                    };
                    let Some(out) = &run.output else { continue };
                    let over = self.strategy_overrides(&run.file);
                    let props = Props::from_json(
                        &opentern_engine::merge_json(&run.props, &over),
                        self.chart.symbol.tick_size,
                    );
                    replay(props, &out.intents, &self.chart.bars)
                }
            };
            let report = Report::new(&broker, &self.chart.bars);
            runs.push(StrategyRun {
                uid: s.uid,
                name: s.meta().short.to_string(),
                broker,
                report,
            });
        }
        self.strategies.runs = runs;
        let visible = |uid: u64| self.chart.studies.iter().any(|s| s.uid == uid && s.visible);
        self.chart.marks = self
            .strategies
            .current()
            .filter(|r| visible(r.uid))
            .map(|r| Arc::new(marks(&r.broker)));
    }

    pub(crate) fn report_tab(&mut self, ui: &mut Ui, t: &Theme) {
        if self.strategies.runs.is_empty() {
            crate::widgets::empty_state(
                ui,
                t,
                crate::icons::ph::CHART_LINE_UP,
                "No strategy on the chart",
                "Add a strategy to see its report. A strategy is a script that starts with strategy(…); \
                 make one in the Script Editor.",
            );
            return;
        }
        let narrow = ui.available_width() < 980.0;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            if self.strategies.runs.len() > 1 {
                let current = self.strategies.current().map(|r| (r.uid, r.name.clone()));
                crate::widgets::combo("report-strategy")
                    .selected_text(current.as_ref().map(|c| c.1.as_str()).unwrap_or(""))
                    .show_ui(ui, |ui| {
                        for r in &self.strategies.runs {
                            if crate::widgets::option(ui, current.as_ref().map(|c| c.0) == Some(r.uid), &r.name)
                                .clicked()
                            {
                                self.strategies.selected = Some(r.uid);
                                self.strategies.key = (0, 0, 0);
                            }
                        }
                    });
                ui.add_space(8.0);
            } else if let Some(r) = self.strategies.current()
                && !narrow
            {
                ui.label(egui::RichText::new(&r.name).font(theme::semibold(13.0)));
                ui.add_space(10.0);
            }
            for tab in ReportTab::ALL {
                let title = if narrow { tab.short() } else { tab.title() };
                if small_button(ui, t, title, self.strategies.tab == tab).clicked() {
                    self.strategies.tab = tab;
                }
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                // Short of room, the two buttons here become icons.
                let settings = if narrow {
                    crate::widgets::tool_button(ui, t, crate::icons::ph::GEAR_SIX, "", false, 26.0)
                } else {
                    small_button(ui, t, "Strategy settings", false)
                };
                if settings
                    .on_hover_text("Strategy settings: capital, order size, commission, slippage and margin")
                    .clicked()
                    && let Some(uid) = self.strategies.current().map(|r| r.uid)
                {
                    self.open_strategy_props(uid);
                }
                if self.strategies.tab == ReportTab::List {
                    let export = if narrow {
                        crate::widgets::tool_button(ui, t, crate::icons::ph::DOWNLOAD_SIMPLE, "", false, 26.0)
                    } else {
                        small_button(ui, t, "Export", false)
                    };
                    ui.add_space(4.0);
                    if export
                        .on_hover_text("Export: save the list of trades as a CSV file")
                        .clicked()
                    {
                        self.export_trades();
                    }
                    if !self.strategies.status.is_empty() {
                        ui.add(
                            egui::Label::new(egui::RichText::new(&self.strategies.status).color(t.text3).size(11.5))
                                .truncate(),
                        );
                    }
                }
            });
        });
        ui.add_space(8.0);
        let cur = self.strategies.current().map(|r| r.uid);
        let Some(run) = self.strategies.runs.iter().find(|r| Some(r.uid) == cur) else {
            return;
        };
        let ccy = if self.chart.symbol.currency.is_empty() {
            "USD".to_string()
        } else {
            self.chart.symbol.currency.clone()
        };
        let ctx = Fmt {
            ccy,
            tz: self.chart.tz.clone(),
        };
        match self.strategies.tab {
            ReportTab::Overview => overview(ui, t, run, &self.chart.bars, &ctx),
            ReportTab::Performance => performance(ui, t, &run.report, &ctx),
            ReportTab::Trades => trades_analysis(ui, t, &run.report, &ctx),
            ReportTab::Ratios => ratios(ui, t, &run.report, &ctx),
            ReportTab::List => list_of_trades(ui, t, &run.broker, &ctx),
            ReportTab::Properties => {
                let sym = self.chart.symbol.id.clone();
                let tf = self.chart.tf.label();
                properties(ui, t, run, &self.chart.bars, &sym, &tf, &ctx)
            }
        }
    }

    fn export_trades(&mut self) {
        let Some(run) = self.strategies.current() else { return };
        let dir = self.store.dir().join("exports");
        let name: String = run
            .name
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || c == ' ' || c == '-' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        let path = dir.join(format!("{name} trades.csv"));
        let csv = trades_csv(&run.broker, &self.chart.tz);
        let result = std::fs::create_dir_all(&dir).and_then(|_| std::fs::write(&path, csv));
        self.strategies.status = match result {
            Ok(()) => format!("Saved to {}", path.display()),
            Err(e) => format!("Couldn't save: {e}"),
        };
    }
}

/// TradingView's "List of trades" columns, one row per entry and exit.
pub fn trades_csv(b: &Broker, tz: &jiff::tz::TimeZone) -> String {
    let mut out =
        String::from("Trade #,Type,Signal,Date/Time,Price,Contracts,Profit,Profit %,Cum. Profit,Run-up,Drawdown\n");
    let mut cum = 0.0;
    let q = |s: &str| format!("\"{}\"", s.replace('"', "\"\""));
    for (k, tr) in b.trades.iter().enumerate() {
        cum += tr.pnl;
        let side = if tr.long { "long" } else { "short" };
        let pct = pct_of_entry(tr);
        for (kind, signal, time, price) in [
            ("Entry", &tr.entry_comment, tr.entry_time, tr.entry_price),
            ("Exit", &tr.exit_comment, tr.exit_time, tr.exit_price),
        ] {
            let signal = if signal.is_empty() {
                if kind == "Entry" { &tr.entry_id } else { &tr.exit_id }
            } else {
                signal
            };
            out += &format!(
                "{},{kind} {side},{},{},{price},{},{:.2},{:.2},{:.2},{:.2},{:.2}\n",
                k + 1,
                q(signal),
                time_text(time, tz),
                qty_text(tr.qty),
                tr.pnl,
                pct,
                cum,
                tr.runup,
                tr.drawdown,
            );
        }
    }
    out
}

fn pct_of_entry(t: &Trade) -> f64 {
    let base = t.entry_price * t.qty;
    if base == 0.0 { 0.0 } else { t.pnl / base * 100.0 }
}

pub(crate) fn time_text(ms: i64, tz: &jiff::tz::TimeZone) -> String {
    jiff::Timestamp::from_millisecond(ms)
        .map(|ts| ts.to_zoned(tz.clone()).strftime("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_default()
}

pub(crate) struct Fmt {
    pub ccy: String,
    pub tz: jiff::tz::TimeZone,
}

impl Fmt {
    pub fn money(&self, v: f64) -> String {
        if !v.is_finite() {
            return "—".into();
        }
        format!("{} {}", grouped(v, 2), self.ccy)
    }
}

/// 1234567.891 → "1,234,567.89"; negatives with a real minus sign.
pub(crate) fn grouped(v: f64, dec: usize) -> String {
    let s = format!("{:.*}", dec, v.abs());
    let (int, frac) = s.split_once('.').unwrap_or((&s, ""));
    let mut g = String::new();
    for (i, c) in int.chars().enumerate() {
        if i > 0 && (int.len() - i).is_multiple_of(3) {
            g.push(',');
        }
        g.push(c);
    }
    let sign = if v < 0.0 && s.chars().any(|c| c.is_ascii_digit() && c != '0') {
        "−"
    } else {
        ""
    };
    if frac.is_empty() {
        format!("{sign}{g}")
    } else {
        format!("{sign}{g}.{frac}")
    }
}

pub(crate) fn pct(v: f64) -> String {
    if v.is_finite() {
        format!("{}%", grouped(v, 2))
    } else {
        "—".into()
    }
}

pub(crate) fn num(v: f64, dec: usize) -> String {
    if v.is_finite() { grouped(v, dec) } else { "—".into() }
}

pub(crate) fn tone(t: &Theme, v: f64) -> Color32 {
    if v > 0.0 {
        t.pos
    } else if v < 0.0 {
        t.neg
    } else {
        t.text
    }
}

// ------------------------------------------------------------- overview

fn overview(ui: &mut Ui, t: &Theme, run: &StrategyRun, bars: &[Bar], f: &Fmt) {
    let r = &run.report;
    let cap = r.initial_capital;
    let net = r.all.net_profit;
    let mut cards: Vec<(&str, String, String, Color32)> = vec![
        ("Total P&L", f.money(net), pct(net / cap * 100.0), tone(t, net)),
        (
            "Max drawdown",
            f.money(r.max_drawdown_intrabar),
            pct(r.max_drawdown_intrabar_pct),
            t.text,
        ),
        ("Total trades", r.all.total_trades.to_string(), String::new(), t.text),
        (
            "Profitable trades",
            pct(r.all.percent_profitable),
            format!("{}/{}", r.all.winners, r.all.total_trades),
            t.text,
        ),
        ("Profit factor", num(r.all.profit_factor, 3), String::new(), t.text),
    ];
    if r.open_trades > 0 {
        cards.push(("Open P&L", f.money(r.open_pnl), String::new(), tone(t, r.open_pnl)));
    }
    // A grid that fits the panel: as many cards per row as there's room for.
    // Short panels get one-line cards so the equity curve keeps its room.
    let compact = ui.available_height() < 320.0;
    let gap = if compact { 8.0 } else { 10.0 };
    let min_w = if compact { 200.0 } else { 170.0 };
    let avail = ui.available_width();
    let per_row = (((avail + gap) / (min_w + gap)).floor() as usize).clamp(1, cards.len());
    let w = (avail - gap * (per_row - 1) as f32) / per_row as f32;
    for row in cards.chunks(per_row) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for (label, value, sub, c) in row {
                kpi(ui, t, w, compact, label, value, sub, *c);
            }
        });
        ui.add_space(gap * 0.6);
    }
    ui.add_space(4.0);
    let h = ui.available_height();
    if h < 90.0 {
        ui.label(egui::RichText::new("Make the panel taller to see the equity curve.").color(t.text3));
        return;
    }
    let size = vec2(ui.available_width(), h);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::hover());
    equity_chart(ui, t, &run.broker, bars, rect, resp.hover_pos(), f);
}

/// A headline number in a card of width `w` (one line when `compact`).
#[allow(clippy::too_many_arguments)]
pub(crate) fn kpi(ui: &mut Ui, t: &Theme, w: f32, compact: bool, label: &str, value: &str, sub: &str, color: Color32) {
    let (r, resp) = ui.allocate_exact_size(vec2(w, if compact { 34.0 } else { 54.0 }), Sense::hover());
    let p = ui.painter_at(r);
    p.rect(
        r,
        CornerRadius::same(8),
        t.card,
        Stroke::new(1.0, t.border),
        egui::StrokeKind::Inside,
    );
    if compact {
        // The value keeps its room (smaller if need be); the label is cut
        // short before it.
        let room = r.width() - 20.0;
        let mut g = p.layout_no_wrap(value.to_string(), theme::semibold(13.0), color);
        for size in [12.0, 11.0] {
            if g.size().x <= room * 0.7 {
                break;
            }
            g = p.layout_no_wrap(value.to_string(), theme::semibold(size), color);
        }
        let vw = g.size().x;
        p.galley(pos2(r.right() - 10.0 - vw, r.center().y - g.size().y / 2.0), g, color);
        let font = theme::regular(11.5);
        let shown = crate::format::elide_to(&p, label, &font, (room - vw - 10.0).max(0.0));
        p.text(
            r.left_center() + vec2(10.0, 0.0),
            Align2::LEFT_CENTER,
            shown,
            font,
            t.text3,
        );
        if !sub.is_empty() {
            resp.on_hover_text(sub);
        }
        return;
    }
    // Everything stays inside the card: the value steps down in size until
    // it fits; the second figure goes beside it, else on the label's line,
    // else into the tooltip.
    let room = r.width() - 24.0;
    let label_g = p.layout_no_wrap(label.to_string(), theme::regular(11.5), t.text3);
    let label_w = label_g.size().x;
    p.galley(r.left_top() + vec2(12.0, 9.0), label_g, t.text3);
    let mut g = p.layout_no_wrap(value.to_string(), theme::semibold(15.0), color);
    for size in [14.0, 13.0, 12.0, 11.0] {
        if g.size().x <= room {
            break;
        }
        g = p.layout_no_wrap(value.to_string(), theme::semibold(size), color);
    }
    let vw = g.size().x;
    let vy = 27.0 + (15.0 - g.size().y.min(15.0)) / 2.0;
    p.galley(r.left_top() + vec2(12.0, vy), g, color);
    if !sub.is_empty() {
        let sub_g = p.layout_no_wrap(sub.to_string(), theme::regular(11.5), t.text2);
        let sw = sub_g.size().x;
        if vw + 8.0 + sw <= room {
            p.galley(r.left_top() + vec2(12.0 + vw + 8.0, 30.0), sub_g, t.text2);
        } else if label_w + 10.0 + sw <= room {
            p.galley(pos2(r.right() - 12.0 - sw, r.top() + 9.0), sub_g, t.text2);
        } else {
            resp.on_hover_text(sub);
        }
    }
}

/// Equity (accent), buy & hold (muted) and drawdown (under the zero line
/// of its own strip), with a hover read-out.
fn equity_chart(ui: &Ui, t: &Theme, b: &Broker, bars: &[Bar], rect: Rect, hover: Option<egui::Pos2>, f: &Fmt) {
    let p = ui.painter_at(rect);
    let n = b.equity.len().min(bars.len());
    if n < 2 {
        return;
    }
    let cap = b.props.initial_capital;
    let first_open = bars[0].open;
    let hold: Vec<f64> = bars[..n]
        .iter()
        .map(|x| {
            if first_open > 0.0 {
                cap * x.close / first_open
            } else {
                cap
            }
        })
        .collect();
    let mut peak = f64::MIN;
    let dd: Vec<f64> = b.equity[..n]
        .iter()
        .map(|e| {
            peak = peak.max(*e);
            peak - e
        })
        .collect();
    let max_dd = dd.iter().cloned().fold(0.0, f64::max);
    let right_gutter = 86.0;
    let plot = Rect::from_min_max(rect.min, pos2(rect.right() - right_gutter, rect.bottom()));
    let dd_h = (plot.height() * 0.22).min(70.0);
    let eq_rect = Rect::from_min_max(plot.min, pos2(plot.right(), plot.bottom() - dd_h - 6.0));
    let dd_rect = Rect::from_min_max(pos2(plot.left(), plot.bottom() - dd_h), plot.max);
    let (lo, hi) = b.equity[..n]
        .iter()
        .chain(&hold)
        .fold((f64::MAX, f64::MIN), |(l, h), v| (l.min(*v), h.max(*v)));
    let pad = ((hi - lo) * 0.06).max(1e-9);
    let (lo, hi) = (lo - pad, hi + pad);
    let xs = |i: usize| plot.left() + plot.width() * i as f32 / (n - 1) as f32;
    let ys = |v: f64| eq_rect.bottom() - eq_rect.height() * ((v - lo) / (hi - lo)) as f32;
    // Grid and scale.
    let font = egui::FontId::proportional(11.0);
    for k in 0..4 {
        let v = lo + (hi - lo) * k as f64 / 4.0;
        let y = ys(v);
        p.hline(eq_rect.x_range(), y, Stroke::new(1.0, t.border.gamma_multiply(0.6)));
        // The lowest label would sit on the drawdown strip's.
        if k == 0 {
            continue;
        }
        p.text(
            pos2(plot.right() + 8.0, y),
            Align2::LEFT_CENTER,
            grouped(v, 0),
            font.clone(),
            t.text3,
        );
    }
    let base = ys(cap);
    p.hline(eq_rect.x_range(), base, Stroke::new(1.0, t.text3.gamma_multiply(0.5)));
    // Area under equity, tinted by gain or loss against the starting capital.
    let line: Vec<egui::Pos2> = (0..n).map(|i| pos2(xs(i), ys(b.equity[i]))).collect();
    let mut mesh = egui::Mesh::default();
    for i in 0..n - 1 {
        let (a, c) = (line[i], line[i + 1]);
        let col = if b.equity[i] >= cap { t.pos } else { t.neg }.gamma_multiply(0.12);
        let idx = mesh.vertices.len() as u32;
        mesh.colored_vertex(a, col);
        mesh.colored_vertex(c, col);
        mesh.colored_vertex(pos2(c.x, base), col);
        mesh.colored_vertex(pos2(a.x, base), col);
        mesh.add_triangle(idx, idx + 1, idx + 2);
        mesh.add_triangle(idx, idx + 2, idx + 3);
    }
    p.add(mesh);
    let hold_line: Vec<egui::Pos2> = (0..n).map(|i| pos2(xs(i), ys(hold[i]))).collect();
    p.add(egui::Shape::line(hold_line, Stroke::new(1.0, t.text3)));
    p.add(egui::Shape::line(line.clone(), Stroke::new(1.6, t.accent)));
    // Drawdown strip.
    if max_dd > 0.0 {
        let mut mesh = egui::Mesh::default();
        let col = t.neg.gamma_multiply(0.55);
        for (i, d) in dd.iter().enumerate() {
            if *d <= 0.0 {
                continue;
            }
            let h = dd_rect.height() * (*d / max_dd) as f32;
            let w = (plot.width() / n as f32).max(1.0);
            let r = Rect::from_min_size(pos2(xs(i) - w / 2.0, dd_rect.top()), vec2(w, h));
            mesh.add_colored_rect(r, col);
        }
        p.add(mesh);
        p.text(
            pos2(plot.right() + 8.0, dd_rect.bottom() - 6.0),
            Align2::LEFT_CENTER,
            format!("−{}", grouped(max_dd, 0)),
            font.clone(),
            t.neg,
        );
    }
    p.hline(dd_rect.x_range(), dd_rect.top(), Stroke::new(1.0, t.border));
    // Legend.
    let legend = [("Equity", t.accent), ("Buy & hold", t.text3), ("Drawdown", t.neg)];
    let mut x = eq_rect.left() + 6.0;
    for (name, c) in legend {
        p.rect_filled(
            Rect::from_center_size(pos2(x + 5.0, eq_rect.top() + 9.0), vec2(10.0, 3.0)),
            1.0,
            c,
        );
        let g = p.layout_no_wrap(name.to_string(), font.clone(), t.text2);
        let w = g.size().x;
        p.galley(pos2(x + 14.0, eq_rect.top() + 2.0), g, t.text2);
        x += w + 28.0;
    }
    // Hover read-out.
    if let Some(h) = hover.filter(|h| plot.contains(*h)) {
        let i = (((h.x - plot.left()) / plot.width()) * (n - 1) as f32)
            .round()
            .clamp(0.0, (n - 1) as f32) as usize;
        let xx = xs(i);
        p.vline(xx, plot.y_range(), Stroke::new(1.0, t.text3.gamma_multiply(0.6)));
        p.circle_filled(line[i], 3.5, t.accent);
        let text = format!(
            "{}\nEquity  {}\nBuy & hold  {}\nDrawdown  {}",
            time_text(bars[i].time, &f.tz),
            f.money(b.equity[i]),
            f.money(hold[i]),
            f.money(-dd[i]),
        );
        let g = p.layout(text, font, t.text, 220.0);
        let size = g.size() + vec2(16.0, 12.0);
        let mut at = pos2(xx + 12.0, eq_rect.top() + 24.0);
        if at.x + size.x > plot.right() {
            at.x = xx - 12.0 - size.x;
        }
        let r = Rect::from_min_size(at, size);
        p.rect(
            r,
            CornerRadius::same(6),
            t.elevated,
            Stroke::new(1.0, t.border),
            egui::StrokeKind::Inside,
        );
        p.galley(r.min + vec2(8.0, 6.0), g, t.text);
    }
}

// ------------------------------------------------------------- tables

/// A metrics table: a label column and value columns (All / Long / Short).
pub(crate) fn table(ui: &mut Ui, t: &Theme, id: &str, headers: &[&str], rows: &[(&str, Vec<(String, Color32)>)]) {
    egui::ScrollArea::vertical()
        .id_salt(id)
        .auto_shrink([false; 2])
        .show(ui, |ui| {
            let w = ui.available_width();
            let label_w = (w * 0.34).clamp(200.0, 320.0);
            let col_w = ((w - label_w) / headers.len().max(1) as f32).min(200.0);
            let row_h = 28.0;
            let head = |ui: &mut Ui| {
                let (r, _) = ui.allocate_exact_size(vec2(w, row_h), Sense::hover());
                for (k, h) in headers.iter().enumerate() {
                    let x = r.left() + label_w + col_w * (k + 1) as f32 - 8.0;
                    ui.painter().text(
                        pos2(x, r.center().y),
                        Align2::RIGHT_CENTER,
                        *h,
                        theme::medium(12.0),
                        t.text3,
                    );
                }
                ui.painter().hline(r.x_range(), r.bottom(), Stroke::new(1.0, t.border));
            };
            head(ui);
            for (label, vals) in rows {
                let (r, resp) = ui.allocate_exact_size(vec2(w, row_h), Sense::hover());
                let p = ui.painter();
                if resp.hovered() {
                    p.rect_filled(r, CornerRadius::same(4), t.hover_wash());
                }
                p.text(
                    pos2(r.left() + 8.0, r.center().y),
                    Align2::LEFT_CENTER,
                    *label,
                    theme::regular(13.0),
                    t.text2,
                );
                for (k, (v, c)) in vals.iter().enumerate() {
                    let x = r.left() + label_w + col_w * (k + 1) as f32 - 8.0;
                    p.text(pos2(x, r.center().y), Align2::RIGHT_CENTER, v, theme::medium(13.0), *c);
                }
                p.hline(r.x_range(), r.bottom(), Stroke::new(1.0, t.border.gamma_multiply(0.5)));
            }
        });
}

fn performance(ui: &mut Ui, t: &Theme, r: &Report, f: &Fmt) {
    let cap = r.initial_capital;
    let sides = [&r.all, &r.long, &r.short];
    let three = |g: &dyn Fn(&opentern_engine::report::Side) -> f64, money: bool| -> Vec<(String, Color32)> {
        sides
            .iter()
            .map(|s| {
                let v = g(s);
                (if money { f.money(v) } else { num(v, 2) }, t.text)
            })
            .collect()
    };
    let signed = |v: f64| (f.money(v), tone(t, v));
    let one = |s: String| vec![(s, t.text), (String::new(), t.text), (String::new(), t.text)];
    let rows: Vec<(&str, Vec<(String, Color32)>)> = vec![
        ("Initial capital", one(f.money(cap))),
        (
            "Open P&L",
            vec![signed(r.open_pnl), (String::new(), t.text), (String::new(), t.text)],
        ),
        ("Net P&L", sides.iter().map(|s| signed(s.net_profit)).collect()),
        (
            "Net P&L %",
            sides
                .iter()
                .map(|s| (pct(s.net_profit / cap * 100.0), tone(t, s.net_profit)))
                .collect(),
        ),
        ("Gross profit", three(&|s| s.gross_profit, true)),
        ("Gross loss", three(&|s| s.gross_loss, true)),
        ("Profit factor", three(&|s| s.profit_factor, false)),
        ("Commission paid", three(&|s| s.commission, true)),
        ("Expected payoff", three(&|s| s.avg_trade, true)),
        (
            "Buy & hold return",
            one(format!("{}  {}", f.money(r.buy_hold_pnl), pct(r.buy_hold_pct))),
        ),
        (
            "Strategy outperformance",
            vec![
                signed(r.outperformance),
                (String::new(), t.text),
                (String::new(), t.text),
            ],
        ),
        (
            "Max equity run-up",
            one(format!(
                "{}  {}",
                f.money(r.max_runup_intrabar),
                pct(r.max_runup_intrabar_pct)
            )),
        ),
        (
            "Max equity drawdown",
            one(format!(
                "{}  {}",
                f.money(r.max_drawdown_intrabar),
                pct(r.max_drawdown_intrabar_pct)
            )),
        ),
        ("Max contracts held", three(&|s| s.max_contracts, false)),
    ];
    table(ui, t, "perf", &["All", "Long", "Short"], &rows);
}

fn trades_analysis(ui: &mut Ui, t: &Theme, r: &Report, f: &Fmt) {
    let sides = [&r.all, &r.long, &r.short];
    let col = |g: &dyn Fn(&opentern_engine::report::Side) -> String| -> Vec<(String, Color32)> {
        sides.iter().map(|s| (g(s), t.text)).collect()
    };
    let rows: Vec<(&str, Vec<(String, Color32)>)> = vec![
        ("Total trades", col(&|s| s.total_trades.to_string())),
        (
            "Total open trades",
            vec![
                (r.open_trades.to_string(), t.text),
                (String::new(), t.text),
                (String::new(), t.text),
            ],
        ),
        ("Winning trades", col(&|s| s.winners.to_string())),
        ("Losing trades", col(&|s| s.losers.to_string())),
        ("Even trades", col(&|s| s.even.to_string())),
        ("Percent profitable", col(&|s| pct(s.percent_profitable))),
        ("Avg P&L", col(&|s| f.money(s.avg_trade))),
        ("Avg P&L %", col(&|s| pct(s.avg_trade_pct))),
        ("Avg winning trade", col(&|s| f.money(s.avg_win))),
        ("Avg losing trade", col(&|s| f.money(s.avg_loss))),
        ("Ratio avg win / avg loss", col(&|s| num(s.win_loss_ratio, 3))),
        ("Largest winning trade", col(&|s| f.money(s.largest_win))),
        (
            "Largest winning trade, % of gross profit",
            col(&|s| pct(s.largest_win_pct_of_gross)),
        ),
        ("Largest losing trade", col(&|s| f.money(s.largest_loss))),
        (
            "Largest losing trade, % of gross loss",
            col(&|s| pct(s.largest_loss_pct_of_gross)),
        ),
        ("Avg # bars in trades", col(&|s| num(s.avg_bars, 0))),
        ("Avg # bars in winning trades", col(&|s| num(s.avg_bars_win, 0))),
        ("Avg # bars in losing trades", col(&|s| num(s.avg_bars_loss, 0))),
    ];
    table(ui, t, "trades", &["All", "Long", "Short"], &rows);
}

fn ratios(ui: &mut Ui, t: &Theme, r: &Report, f: &Fmt) {
    let v = |s: String| vec![(s, t.text)];
    let rows: Vec<(&str, Vec<(String, Color32)>)> = vec![
        ("Sharpe ratio", v(num(r.sharpe, 3))),
        ("Sortino ratio", v(num(r.sortino, 3))),
        ("Profit factor", v(num(r.all.profit_factor, 3))),
        ("CAGR", v(pct(r.cagr))),
        ("Return on initial capital", v(pct(r.return_on_capital))),
        ("Account size required", v(f.money(r.account_size_required))),
        ("Return on account size required", v(pct(r.return_on_account_size))),
        (
            "Net profit as % of largest loss",
            v(pct(r.net_profit_pct_of_largest_loss)),
        ),
        ("Return of max drawdown", v(num(r.return_of_max_drawdown, 2))),
        ("Avg equity drawdown", v(f.money(r.avg_drawdown))),
        ("Avg drawdown duration (bars)", v(num(r.avg_drawdown_bars, 0))),
        ("Avg equity run-up", v(f.money(r.avg_runup))),
        ("Avg run-up duration (bars)", v(num(r.avg_runup_bars, 0))),
        ("Avg margin used", v(f.money(r.avg_margin_used))),
        ("Max margin used", v(f.money(r.max_margin_used))),
        ("Margin efficiency", v(pct(r.margin_efficiency))),
        ("Margin calls", v(r.margin_calls.to_string())),
    ];
    table(ui, t, "ratios", &["Value"], &rows);
}

fn list_of_trades(ui: &mut Ui, t: &Theme, b: &Broker, f: &Fmt) {
    if b.trades.is_empty() {
        ui.label(egui::RichText::new("No closed trades yet.").color(t.text3));
        return;
    }
    let cols: [(&str, f32, bool); 10] = [
        ("Trade #", 64.0, false),
        ("Type", 92.0, false),
        ("Signal", 190.0, false),
        ("Date/Time", 136.0, false),
        ("Price", 120.0, true),
        ("Contracts", 110.0, true),
        ("Profit", 190.0, true),
        ("Cum. profit", 150.0, true),
        ("Run-up", 130.0, true),
        ("Drawdown", 130.0, true),
    ];
    let total: f32 = cols.iter().map(|c| c.1).sum();
    let row_h = 22.0;
    let mut cum = Vec::with_capacity(b.trades.len());
    let mut acc = 0.0;
    for tr in &b.trades {
        acc += tr.pnl;
        cum.push(acc);
    }
    egui::ScrollArea::horizontal().id_salt("trades-h").show(ui, |ui| {
        ui.set_min_width(total);
        let (hr, _) = ui.allocate_exact_size(vec2(total, 26.0), Sense::hover());
        let mut x = hr.left();
        for (name, w, right) in cols {
            let (pos, al) = if right {
                (pos2(x + w - 8.0, hr.center().y), Align2::RIGHT_CENTER)
            } else {
                (pos2(x + 8.0, hr.center().y), Align2::LEFT_CENTER)
            };
            ui.painter().text(pos, al, name, theme::medium(12.0), t.text3);
            x += w;
        }
        ui.painter()
            .hline(hr.x_range(), hr.bottom(), Stroke::new(1.0, t.border));
        let n = b.trades.len();
        // Newest first; two lines per trade (exit, then entry).
        egui::ScrollArea::vertical()
            .id_salt("trades-v")
            .auto_shrink([false; 2])
            .show_rows(ui, row_h * 2.0, n, |ui, range| {
                for k in range {
                    let idx = n - 1 - k;
                    let tr = &b.trades[idx];
                    let (r, resp) = ui.allocate_exact_size(vec2(total, row_h * 2.0), Sense::hover());
                    let p = ui.painter();
                    if resp.hovered() {
                        p.rect_filled(r, CornerRadius::same(4), t.hover_wash());
                    }
                    let side = if tr.long { "long" } else { "short" };
                    let pnl_c = tone(t, tr.pnl);
                    let lines: [[(String, Color32); 10]; 2] = [
                        [
                            (format!("{}", idx + 1), t.text),
                            (format!("Exit {side}"), t.text2),
                            (
                                if tr.exit_comment.is_empty() {
                                    tr.exit_id.clone()
                                } else {
                                    tr.exit_comment.clone()
                                },
                                t.text2,
                            ),
                            (time_text(tr.exit_time, &f.tz), t.text2),
                            (grouped(tr.exit_price, 2), t.text),
                            (qty_text(tr.qty), t.text),
                            (format!("{}  {}", f.money(tr.pnl), pct(pct_of_entry(tr))), pnl_c),
                            (f.money(cum[idx]), tone(t, cum[idx])),
                            (f.money(tr.runup), t.text2),
                            (f.money(tr.drawdown), t.text2),
                        ],
                        [
                            (String::new(), t.text),
                            (format!("Entry {side}"), t.text2),
                            (
                                if tr.entry_comment.is_empty() {
                                    tr.entry_id.clone()
                                } else {
                                    tr.entry_comment.clone()
                                },
                                t.text2,
                            ),
                            (time_text(tr.entry_time, &f.tz), t.text2),
                            (grouped(tr.entry_price, 2), t.text),
                            (String::new(), t.text),
                            (String::new(), t.text),
                            (String::new(), t.text),
                            (String::new(), t.text),
                            (String::new(), t.text),
                        ],
                    ];
                    for (li, line) in lines.iter().enumerate() {
                        let cy = r.top() + row_h * (li as f32 + 0.5);
                        let mut x = r.left();
                        for ((_, w, right), (text, c)) in cols.iter().zip(line.iter()) {
                            let (pos, al) = if *right {
                                (pos2(x + w - 8.0, cy), Align2::RIGHT_CENTER)
                            } else {
                                (pos2(x + 8.0, cy), Align2::LEFT_CENTER)
                            };
                            // Elided to the column, so nothing runs into the next.
                            let font = theme::regular(12.5);
                            let shown = crate::format::elide_to(p, text, &font, w - 14.0);
                            p.text(pos, al, shown, font, *c);
                            x += w;
                        }
                    }
                    p.hline(r.x_range(), r.bottom(), Stroke::new(1.0, t.border.gamma_multiply(0.5)));
                }
            });
    });
}

#[allow(clippy::too_many_arguments)]
fn properties(ui: &mut Ui, t: &Theme, run: &StrategyRun, bars: &[Bar], sym: &str, tf: &str, f: &Fmt) {
    let p = &run.broker.props;
    let range = match (bars.first(), bars.last()) {
        (Some(a), Some(b)) => format!("{} — {}", time_text(a.time, &f.tz), time_text(b.time, &f.tz)),
        _ => "—".into(),
    };
    let size = match p.qty_type {
        opentern_engine::QtyType::Fixed => format!("{} contracts", qty_text(p.qty_value)),
        opentern_engine::QtyType::Cash => f.money(p.qty_value),
        opentern_engine::QtyType::PercentOfEquity => format!("{}% of equity", num(p.qty_value, 2)),
    };
    let commission = match p.commission {
        opentern_engine::Commission::Percent => format!("{}%", num(p.commission_value, 4)),
        opentern_engine::Commission::CashPerContract => format!("{} per contract", f.money(p.commission_value)),
        opentern_engine::Commission::CashPerOrder => format!("{} per order", f.money(p.commission_value)),
    };
    let v = |s: String| vec![(s, t.text)];
    let rows: Vec<(&str, Vec<(String, Color32)>)> = vec![
        ("Symbol", v(sym.to_string())),
        ("Timeframe", v(tf.to_string())),
        ("Trading range", v(range)),
        ("Bars", v(bars.len().to_string())),
        ("Initial capital", v(f.money(p.initial_capital))),
        ("Order size", v(size)),
        ("Pyramiding", v(format!("{} orders", p.pyramiding.max(1)))),
        ("Commission", v(commission)),
        ("Slippage", v(format!("{} ticks", p.slippage))),
        ("Margin for long positions", v(format!("{}%", num(p.margin_long, 0)))),
        ("Margin for short positions", v(format!("{}%", num(p.margin_short, 0)))),
        (
            "Fill orders on bar close",
            v(if p.process_orders_on_close { "On" } else { "Off" }.into()),
        ),
    ];
    table(ui, t, "props", &["Value"], &rows);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn number_formatting() {
        assert_eq!(grouped(1234567.891, 2), "1,234,567.89");
        assert_eq!(grouped(-12.5, 2), "−12.50");
        assert_eq!(grouped(-0.001, 2), "0.00");
        assert_eq!(pct(f64::NAN), "—");
    }

    #[test]
    fn python_intents_replay_through_the_broker() {
        let bars: Vec<Bar> = (0..6)
            .map(|i| Bar {
                time: i * 60_000,
                open: 10.0 + i as f64,
                high: 11.0 + i as f64,
                low: 9.0 + i as f64,
                close: 10.5 + i as f64,
                volume: 1.0,
            })
            .collect();
        let intents = vec![
            serde_json::json!({"bar": 0, "type": "entry", "id": "L", "long": true, "qty": 2.0}),
            serde_json::json!({"bar": 3, "type": "close", "id": "L"}),
        ];
        let b = replay(Props::default(), &intents, &bars);
        assert_eq!(b.trades.len(), 1);
        assert_eq!((b.trades[0].entry_price, b.trades[0].exit_price), (11.0, 14.0));
        assert_eq!(b.trades[0].qty, 2.0);
        let m = marks(&b);
        assert_eq!(m.fills.len(), 2);
        assert_eq!(m.fills[0].label, "L +2");
        let csv = trades_csv(&b, &jiff::tz::TimeZone::UTC);
        assert_eq!(csv.lines().count(), 3);
        assert!(csv.lines().nth(1).unwrap().starts_with("1,Entry long,\"L\""));
    }
}

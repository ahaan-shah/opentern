//! Strategies on the chart: their broker runs (a Pine strategy's own broker,
//! or a Python strategy's orders replayed through one), the trades drawn on
//! the chart, and the Strategy Report tab with TradingView's layout and
//! metrics (`opentern_engine::Report`; formulas in `docs/metrics.md`).

use crate::app::App;
use crate::chart::{MarkFill, TradeMarks};
use crate::i18n::{msg, nt, tr, trf, trm};
use crate::shell::small_button;
use crate::theme::{self, Theme};
use egui::{Align, Align2, Color32, CornerRadius, Layout, Rect, Sense, Stroke, Ui, pos2, vec2};
use opentern_core::{Bar, Symbol, Timeframe};
use opentern_engine::{Broker, Order, OrderKind, Props, Report, Trade};
use opentern_pine::PineStudy;
use opentern_series::study::Instance;
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
            ReportTab::Overview => nt!("Overview"),
            ReportTab::Performance => nt!("Performance"),
            ReportTab::Trades => nt!("Trades"),
            ReportTab::Ratios => nt!("Ratios"),
            ReportTab::List => nt!("List"),
            ReportTab::Properties => nt!("Properties"),
        }
    }

    fn title(self) -> &'static str {
        match self {
            ReportTab::Overview => nt!("Overview"),
            ReportTab::Performance => nt!("Performance"),
            ReportTab::Trades => nt!("Trades analysis"),
            ReportTab::Ratios => nt!("Risk/performance ratios"),
            ReportTab::List => nt!("List of trades"),
            ReportTab::Properties => nt!("Properties"),
        }
    }
}

#[derive(Default)]
pub struct Strategies {
    pub runs: Vec<StrategyRun>,
    pub(crate) key: (u64, u64, u64, bool),
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

/// Whether the last of `bars` is still forming: the market is open and its
/// period hasn't ended. (A daily stock bar after 16:00, or any bar while
/// replaying, is finished.)
pub(crate) fn last_bar_forming(symbol: &Symbol, tf: Timeframe, bars: &[Bar], replaying: bool) -> bool {
    use opentern_core::calendar::{self, Market, State};
    let Some(last) = bars.last() else { return false };
    if replaying {
        return false;
    }
    let m = Market::of(symbol);
    let now = jiff::Timestamp::now().as_millisecond();
    if calendar::state(m, now) != State::Open {
        return false;
    }
    let end = match tf.millis() {
        Some(ms) if tf.is_intraday() => last.time + ms,
        Some(ms) => calendar::last_close_in(m, last.time, last.time + ms).unwrap_or(last.time + ms),
        // Months: the latest one is still running while the market is open.
        None => return true,
    };
    now < end
}

/// A strategy study's broker as TradingView reports it. A strategy runs
/// once per closed bar, so while the last bar is still forming the script
/// hasn't run on it: its signals don't count yet, but orders placed on the
/// last closed bar fill on it (at its open, or along its path so far).
pub(crate) fn closed_broker(inst: &Instance, bars: &[Bar], forming: bool) -> Option<Broker> {
    if !forming {
        return inst.latest_state().and_then(broker_of).cloned();
    }
    let mut b = inst.state().and_then(broker_of).cloned()?;
    if let Some(i) = bars.len().checked_sub(1) {
        b.open_bar(i, &bars[i]);
        b.close_bar(&bars[i]);
    }
    Some(b)
}

/// A Python strategy's per-bar orders, filled bar by bar by the broker
/// (orders from a still-forming last bar are left out, as above).
pub fn replay(props: Props, intents: &[serde_json::Value], bars: &[Bar], forming: bool) -> Broker {
    let last = if forming {
        bars.len().saturating_sub(1)
    } else {
        bars.len()
    };
    let mut by_bar: Vec<Vec<&serde_json::Value>> = vec![Vec::new(); bars.len()];
    for it in intents {
        if let Some(i) = it.get("bar").and_then(|b| b.as_u64())
            && let Some(v) = by_bar.get_mut(i as usize)
            && (i as usize) < last
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
            // A size that's given and isn't more than 0 orders nothing.
            if ["qty", "qty_percent"]
                .iter()
                .any(|k| num(it, k).is_some_and(|q| q <= 0.0))
            {
                continue;
            }
            let mut o = Order::market(&id, kind, long);
            o.qty = num(it, "qty");
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
        let forming = last_bar_forming(
            &self.chart.symbol,
            self.chart.tf,
            &self.chart.bars,
            self.chart.replay.is_some(),
        );
        let key = (self.chart.data_version, self.chart.changed, py, forming);
        if key == self.strategies.key {
            return;
        }
        self.strategies.key = key;
        let mut runs = Vec::new();
        for s in &self.chart.studies {
            let pine = closed_broker(&s.inst, &self.chart.bars, forming);
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
                        crate::pine_host::min_qty_of(&self.chart.symbol),
                    );
                    replay(props, &out.intents, &self.chart.bars, forming)
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
            crate::widgets::empty_state(ui, t, crate::icons::ph::CHART_LINE_UP, tr("No strategy on the chart"));
            return;
        }
        let narrow = ui.available_width() < 980.0;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            if self.strategies.runs.len() > 1 {
                let current = self.strategies.current().map(|r| (r.uid, r.name.clone()));
                let name = current.as_ref().map(|c| c.1.as_str()).unwrap_or("");
                // A long script name leaves room for the tabs.
                let most = ui.available_width() * 0.4;
                let w = (crate::widgets::widest(ui, theme::medium(13.0), [name]) + 46.0).clamp(70.0, most.max(70.0));
                crate::widgets::combo("report-strategy")
                    .width(w)
                    .truncate()
                    .selected_text(name)
                    .show_ui(ui, |ui| {
                        for r in &self.strategies.runs {
                            if crate::widgets::option(ui, current.as_ref().map(|c| c.0) == Some(r.uid), &r.name)
                                .clicked()
                            {
                                self.strategies.selected = Some(r.uid);
                                self.strategies.key = (0, 0, 0, false);
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
            // The buttons on the right keep their room; the tabs scroll in
            // what's left, so the row never stretches the report.
            let list = self.strategies.tab == ReportTab::List;
            let buttons = if narrow {
                26.0 + if list { 34.0 } else { 0.0 }
            } else {
                let font = theme::medium(12.5);
                let names = [tr("Strategy settings")].into_iter().chain(list.then(|| tr("Export")));
                names
                    .map(|n| crate::widgets::widest(ui, font.clone(), [n]) + 26.0)
                    .sum()
            };
            let room = (ui.available_width() - buttons - 10.0).max(60.0);
            crate::widgets::tab_scroll(ui, "report-tabs", self.strategies.tab, room, |ui, reveal| {
                ui.horizontal(|ui| {
                    for tab in ReportTab::ALL {
                        let title = tr(if narrow { tab.short() } else { tab.title() });
                        let active = self.strategies.tab == tab;
                        let r = small_button(ui, t, title, active);
                        if active && reveal {
                            r.scroll_to_me(Some(Align::Center));
                        }
                        if r.clicked() {
                            self.strategies.tab = tab;
                        }
                    }
                });
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                // Short of room, the two buttons here become icons.
                let settings = if narrow {
                    crate::widgets::tool_button(ui, t, crate::icons::ph::GEAR_SIX, "", false, 26.0)
                } else {
                    small_button(ui, t, tr("Strategy settings"), false)
                };
                if settings
                    .on_hover_text(tr(
                        "Strategy settings: capital, order size, commission, slippage and margin",
                    ))
                    .clicked()
                    && let Some(uid) = self.strategies.current().map(|r| r.uid)
                {
                    self.open_strategy_props(uid);
                }
                if self.strategies.tab == ReportTab::List {
                    let export = if narrow {
                        crate::widgets::tool_button(ui, t, crate::icons::ph::DOWNLOAD_SIMPLE, "", false, 26.0)
                    } else {
                        small_button(ui, t, tr("Export"), false)
                    };
                    ui.add_space(4.0);
                    if export
                        .on_hover_text(tr("Export: save the list of trades as a CSV file"))
                        .clicked()
                    {
                        self.export_trades();
                    }
                    if !self.strategies.status.is_empty() {
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(trm(&self.strategies.status))
                                    .color(t.text3)
                                    .size(11.5),
                            )
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
            Ok(()) => msg!("Saved to {}", path.display()),
            Err(e) => msg!("Couldn't save: {e}"),
        };
    }
}

/// TradingView's "List of trades" columns, one row per entry and exit.
pub fn trades_csv(b: &Broker, tz: &jiff::tz::TimeZone) -> String {
    let mut out =
        String::from("Trade #,Type,Signal,Date/Time,Price,Contracts,Profit,Profit %,Cum. Profit,Run-up,Drawdown\n");
    let mut cum = 0.0;
    let q = |s: &str| {
        let s = opentern_engine::quant::export::cell(s);
        if s.starts_with('"') { s } else { format!("\"{s}\"") }
    };
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
        (tr("Total P&L"), f.money(net), pct(net / cap * 100.0), tone(t, net)),
        (
            tr("Max drawdown"),
            f.money(r.max_drawdown_intrabar),
            pct(r.max_drawdown_intrabar_pct),
            t.text,
        ),
        (
            tr("Total trades"),
            r.all.total_trades.to_string(),
            String::new(),
            t.text,
        ),
        (
            tr("Profitable trades"),
            pct(r.all.percent_profitable),
            format!("{}/{}", r.all.winners, r.all.total_trades),
            t.text,
        ),
        (tr("Profit factor"), num(r.all.profit_factor, 3), String::new(), t.text),
    ];
    if r.open_trades > 0 {
        cards.push((tr("Open P&L"), f.money(r.open_pnl), String::new(), tone(t, r.open_pnl)));
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
        ui.label(egui::RichText::new(tr("Make the panel taller to see the equity curve.")).color(t.text3));
        return;
    }
    let size = vec2(ui.available_width(), h);
    equity_chart(ui, t, &run.broker, bars, size, f);
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

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum PerformanceScale {
    #[default]
    Regular,
    Percent,
}

impl PerformanceScale {
    fn axis_label(self, value: f64) -> String {
        match self {
            Self::Regular => num(value, 0),
            Self::Percent => pct(value),
        }
    }

    fn tooltip_value(self, value: f64, f: &Fmt) -> String {
        match self {
            Self::Regular => f.money(value),
            Self::Percent => pct(value),
        }
    }
}

/// Equity, buy & hold and drawdown, with a clickable legend and hover read-out.
fn equity_chart(ui: &mut Ui, t: &Theme, b: &Broker, bars: &[Bar], size: egui::Vec2, f: &Fmt) {
    let (id, rect) = ui.allocate_space(size);
    let p = ui.painter_at(rect);
    let end = b.equity.len().min(bars.len());
    let start = b.fills.first().map_or(0, |fill| fill.bar);
    if start >= end || (end == 1 && b.fills.is_empty()) {
        return;
    }
    let bars = &bars[start..end];
    let values = &b.equity[start..end];
    let n = bars.len();

    let cap = b.props.initial_capital;
    let scale_id = ui.make_persistent_id("equity-chart-scale");
    let percent_available = cap > 0.0;
    let mut scale = if percent_available {
        ui.data(|d| d.get_temp::<PerformanceScale>(scale_id))
            .unwrap_or_default()
    } else {
        PerformanceScale::Regular
    };
    let header = Rect::from_min_size(rect.min, vec2(rect.width(), 26.0));
    let mut controls = ui.new_child(
        egui::UiBuilder::new()
            .id_salt(scale_id)
            .max_rect(header)
            .layout(Layout::right_to_left(Align::Center)),
    );
    controls.spacing_mut().item_spacing.x = 0.0;
    for (choice, label, enabled) in [
        (PerformanceScale::Percent, tr("Percent"), percent_available),
        (PerformanceScale::Regular, tr("Regular"), true),
    ] {
        let response = controls
            .add_enabled_ui(enabled, |ui| small_button(ui, t, label, scale == choice))
            .inner;
        if response.clicked() && scale != choice {
            scale = choice;
            ui.data_mut(|d| d.insert_temp(scale_id, scale));
            ui.ctx().request_repaint();
        }
        response.widget_info(|| {
            egui::WidgetInfo::selected(
                egui::WidgetType::SelectableLabel,
                response.enabled(),
                scale == choice,
                label,
            )
        });
    }

    let font = egui::FontId::proportional(11.0);
    let legend_id = ui.make_persistent_id("equity-chart-legend");
    let previous = ui.data(|d| d.get_temp(legend_id)).unwrap_or([true; 3]);
    let mut visible = previous;
    let legend_rect = Rect::from_min_max(header.min, pos2(controls.cursor().right(), header.bottom()));
    let mut x = rect.left() + 6.0;
    let legend = [
        (0, tr("Equity"), t.accent),
        (1, tr("Buy & hold"), t.text3),
        (2, tr("Drawdown"), t.neg),
    ]
    .map(|(i, name, c)| {
        let g = p.layout_no_wrap(name.to_string(), font.clone(), t.text2);
        let at = x;
        let hit =
            Rect::from_min_size(pos2(x, header.top()), vec2(g.size().x + 18.0, header.height())).intersect(legend_rect);
        let mut resp = ui
            .interact(hit, legend_id.with(i), Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        if resp.clicked() {
            visible[i] = !visible[i];
            resp.mark_changed();
        }
        resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Checkbox, ui.is_enabled(), visible[i], name));
        x += g.size().x + 28.0;
        (at, g, c, visible[i])
    });
    if visible != previous {
        ui.data_mut(|d| d.insert_temp(legend_id, visible));
    }
    let [equity_on, hold_on, drawdown_on] = visible;
    let has_curves = equity_on || hold_on;

    let equity = equity_on.then(|| {
        values
            .iter()
            .map(|e| match scale {
                PerformanceScale::Regular => e - cap,
                PerformanceScale::Percent => (e - cap) / cap * 100.0,
            })
            .collect::<Vec<_>>()
    });
    let hold = hold_on.then(|| {
        let first_open = bars[0].open;
        bars.iter()
            .map(|x| {
                let change = if first_open > 0.0 {
                    x.close / first_open - 1.0
                } else {
                    0.0
                };
                match scale {
                    PerformanceScale::Regular => cap * change,
                    PerformanceScale::Percent => change * 100.0,
                }
            })
            .collect::<Vec<_>>()
    });
    let dd = drawdown_on.then(|| {
        // Keep the drawdown of each visible bar relative to the original equity peak.
        let mut peak = b.equity[..start].iter().copied().fold(f64::MIN, f64::max);
        values
            .iter()
            .map(|e| {
                peak = peak.max(*e);
                match scale {
                    PerformanceScale::Regular => Some(e - peak),
                    PerformanceScale::Percent => (peak > 0.0).then(|| (e - peak) / peak * 100.0),
                }
            })
            .collect::<Vec<_>>()
    });
    let right_gutter = 86.0;
    let plot = Rect::from_min_max(
        pos2(rect.left(), header.bottom()),
        pos2(rect.right() - right_gutter, rect.bottom()),
    );
    let dd_h = (plot.height() * 0.22).min(70.0);
    let xs = |i: usize| plot.left() + plot.width() * i as f32 / (n - 1).max(1) as f32;
    let hover = ui.interact(plot, id, Sense::hover()).hover_pos();
    let hover_index = hover
        .filter(|h| (has_curves || drawdown_on) && plot.contains(*h))
        .map(|h| {
            (((h.x - plot.left()) / plot.width()) * (n - 1) as f32)
                .round()
                .clamp(0.0, (n - 1) as f32) as usize
        });

    let mut equity_marker = None;
    let zero_stroke = Stroke::new(1.5, t.text3);
    if has_curves {
        let eq_rect = if drawdown_on {
            Rect::from_min_max(plot.min, pos2(plot.right(), plot.bottom() - dd_h - 6.0))
        } else {
            plot
        };
        let (lo, hi) = equity
            .as_deref()
            .into_iter()
            .chain(hold.as_deref())
            .flatten()
            .fold((0.0_f64, 0.0_f64), |(l, h), v| (l.min(*v), h.max(*v)));
        let pad = ((hi - lo) * 0.06).max(1e-9);
        let (lo, hi) = (lo - pad, hi + pad);
        let ys = |v: f64| eq_rect.bottom() - eq_rect.height() * ((v - lo) / (hi - lo)) as f32;
        let base = ys(0.0);
        // Grid and scale.
        for k in 0..4 {
            let v = lo + (hi - lo) * k as f64 / 4.0;
            let y = ys(v);
            p.hline(eq_rect.x_range(), y, Stroke::new(1.0, t.border.gamma_multiply(0.6)));
            // The lowest label is too close to the bottom edge or the drawdown scale.
            if k == 0 || (y - base).abs() < 11.0 {
                continue;
            }
            p.text(
                pos2(plot.right() + 8.0, y),
                Align2::LEFT_CENTER,
                scale.axis_label(v),
                font.clone(),
                t.text3,
            );
        }
        let line = equity.as_ref().map(|values| {
            let line: Vec<_> = values.iter().enumerate().map(|(i, v)| pos2(xs(i), ys(*v))).collect();
            let mut mesh = egui::Mesh::default();
            let mut shade = |a: egui::Pos2, c: egui::Pos2, positive| {
                let col = if positive { t.pos } else { t.neg }.gamma_multiply(0.12);
                let idx = mesh.vertices.len() as u32;
                mesh.colored_vertex(a, col);
                mesh.colored_vertex(c, col);
                mesh.colored_vertex(pos2(c.x, base), col);
                mesh.colored_vertex(pos2(a.x, base), col);
                mesh.add_triangle(idx, idx + 1, idx + 2);
                mesh.add_triangle(idx, idx + 2, idx + 3);
            };
            for i in 0..n - 1 {
                let (a, c) = (line[i], line[i + 1]);
                let (v, next) = (values[i], values[i + 1]);
                let positive = v >= 0.0;
                if positive == (next >= 0.0) {
                    shade(a, c, positive);
                } else {
                    let crossing = pos2(egui::lerp(a.x..=c.x, (v / (v - next)) as f32), base);
                    shade(a, crossing, positive);
                    shade(crossing, c, !positive);
                }
            }
            p.add(mesh);
            line
        });
        p.hline(eq_rect.x_range(), base, zero_stroke);
        p.text(
            pos2(plot.right() + 8.0, base),
            Align2::LEFT_CENTER,
            scale.axis_label(0.0),
            font.clone(),
            t.text2,
        );
        let paint_curve = |line: Vec<_>, stroke: Stroke| {
            if let [point] = line.as_slice() {
                p.circle_filled(*point, stroke.width, stroke.color);
            } else {
                p.add(egui::Shape::line(line, stroke));
            }
        };
        if let Some(hold) = &hold {
            let line = hold.iter().enumerate().map(|(i, v)| pos2(xs(i), ys(*v))).collect();
            paint_curve(line, Stroke::new(1.0, t.text3));
        }
        if let Some(line) = line {
            equity_marker = hover_index.map(|i| line[i]);
            paint_curve(line, Stroke::new(1.6, t.accent));
        }
    }
    if let Some(dd) = &dd {
        let dd_rect = if has_curves {
            Rect::from_min_max(pos2(plot.left(), plot.bottom() - dd_h), plot.max)
        } else {
            plot
        };
        let min_dd = dd.iter().flatten().copied().fold(0.0, f64::min);
        if min_dd < 0.0 {
            let mut mesh = egui::Mesh::default();
            let col = t.neg.gamma_multiply(0.55);
            for (i, d) in dd
                .iter()
                .enumerate()
                .filter_map(|(i, d)| d.filter(|d| *d < 0.0).map(|d| (i, d)))
            {
                let h = dd_rect.height() * (d / min_dd) as f32;
                let w = (plot.width() / n as f32).max(1.0);
                let r = Rect::from_min_size(pos2(xs(i) - w / 2.0, dd_rect.top()), vec2(w, h));
                mesh.add_colored_rect(r, col);
            }
            p.add(mesh);
            p.text(
                pos2(plot.right() + 8.0, dd_rect.bottom() - 6.0),
                Align2::LEFT_CENTER,
                scale.axis_label(min_dd),
                font.clone(),
                t.neg,
            );
        }
        p.hline(dd_rect.x_range(), dd_rect.top(), zero_stroke);
        p.text(
            pos2(plot.right() + 8.0, dd_rect.top()),
            Align2::LEFT_CENTER,
            scale.axis_label(0.0),
            font.clone(),
            t.text2,
        );
    }
    // Legend.
    for (x, g, c, on) in legend {
        let mut p = p.with_clip_rect(legend_rect);
        if !on {
            p.multiply_opacity(0.45);
        }
        p.rect_filled(
            Rect::from_center_size(pos2(x + 5.0, header.center().y), vec2(10.0, 3.0)),
            1.0,
            c,
        );
        p.galley(pos2(x + 14.0, header.center().y - g.size().y / 2.0), g, t.text2);
    }
    // Hover read-out.
    if let Some(i) = hover_index {
        let xx = xs(i);
        p.vline(xx, plot.y_range(), Stroke::new(1.0, t.text3.gamma_multiply(0.6)));
        if let Some(marker) = equity_marker {
            p.circle_filled(marker, 3.5, t.accent);
        }
        let time = time_text(bars[i].time, &f.tz);
        let display = |value| scale.tooltip_value(value, f);
        let equity = equity.as_ref().map(|v| display(v[i]));
        let hold = hold.as_ref().map(|v| display(v[i]));
        let dd = dd.as_ref().map(|v| v[i].map_or_else(|| "—".into(), display));
        let text = match (&equity, &hold, &dd) {
            (Some(equity), Some(hold), Some(dd)) => {
                trf!("{}\nEquity  {}\nBuy & hold  {}\nDrawdown  {}", time, equity, hold, dd,)
            }
            _ => {
                let mut text = time;
                for (name, value) in [(tr("Equity"), equity), (tr("Buy & hold"), hold), (tr("Drawdown"), dd)] {
                    if let Some(value) = value {
                        use std::fmt::Write;
                        let _ = write!(text, "\n{name}  {value}");
                    }
                }
                text
            }
        };
        let g = p.layout(text, font, t.text, 220.0);
        let size = g.size() + vec2(16.0, 12.0);
        let mut at = pos2(xx + 12.0, plot.top() + 24.0);
        if at.x + size.x > plot.right() {
            at.x = xx - 12.0 - size.x;
        }
        at.y = at.y.clamp(plot.top(), (plot.bottom() - size.y).max(plot.top()));
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
                p.rect_filled(
                    r,
                    CornerRadius::same(4),
                    crate::motion::with_alpha(t.hover_wash(), crate::motion::hover(&resp)),
                );
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
        (tr("Initial capital"), one(f.money(cap))),
        (
            tr("Open P&L"),
            vec![signed(r.open_pnl), (String::new(), t.text), (String::new(), t.text)],
        ),
        (tr("Net P&L"), sides.iter().map(|s| signed(s.net_profit)).collect()),
        (
            tr("Net P&L %"),
            sides
                .iter()
                .map(|s| (pct(s.net_profit / cap * 100.0), tone(t, s.net_profit)))
                .collect(),
        ),
        (tr("Gross profit"), three(&|s| s.gross_profit, true)),
        (tr("Gross loss"), three(&|s| s.gross_loss, true)),
        (tr("Profit factor"), three(&|s| s.profit_factor, false)),
        (tr("Commission paid"), three(&|s| s.commission, true)),
        (tr("Expected payoff"), three(&|s| s.avg_trade, true)),
        (
            tr("Buy & hold return"),
            one(format!("{}  {}", f.money(r.buy_hold_pnl), pct(r.buy_hold_pct))),
        ),
        (
            tr("Strategy outperformance"),
            vec![
                signed(r.outperformance),
                (String::new(), t.text),
                (String::new(), t.text),
            ],
        ),
        (
            tr("Max equity run-up"),
            one(format!(
                "{}  {}",
                f.money(r.max_runup_intrabar),
                pct(r.max_runup_intrabar_pct)
            )),
        ),
        (
            tr("Max equity drawdown"),
            one(format!(
                "{}  {}",
                f.money(r.max_drawdown_intrabar),
                pct(r.max_drawdown_intrabar_pct)
            )),
        ),
        (tr("Max contracts held"), three(&|s| s.max_contracts, false)),
    ];
    table(ui, t, "perf", &[tr("All"), tr("Long"), tr("Short")], &rows);
}

fn trades_analysis(ui: &mut Ui, t: &Theme, r: &Report, f: &Fmt) {
    let sides = [&r.all, &r.long, &r.short];
    let col = |g: &dyn Fn(&opentern_engine::report::Side) -> String| -> Vec<(String, Color32)> {
        sides.iter().map(|s| (g(s), t.text)).collect()
    };
    let rows: Vec<(&str, Vec<(String, Color32)>)> = vec![
        (tr("Total trades"), col(&|s| s.total_trades.to_string())),
        (
            tr("Total open trades"),
            vec![
                (r.open_trades.to_string(), t.text),
                (String::new(), t.text),
                (String::new(), t.text),
            ],
        ),
        (tr("Winning trades"), col(&|s| s.winners.to_string())),
        (tr("Losing trades"), col(&|s| s.losers.to_string())),
        (tr("Even trades"), col(&|s| s.even.to_string())),
        (tr("Percent profitable"), col(&|s| pct(s.percent_profitable))),
        (tr("Avg P&L"), col(&|s| f.money(s.avg_trade))),
        (tr("Avg P&L %"), col(&|s| pct(s.avg_trade_pct))),
        (tr("Avg winning trade"), col(&|s| f.money(s.avg_win))),
        (tr("Avg losing trade"), col(&|s| f.money(s.avg_loss))),
        (tr("Ratio avg win / avg loss"), col(&|s| num(s.win_loss_ratio, 3))),
        (tr("Largest winning trade"), col(&|s| f.money(s.largest_win))),
        (
            tr("Largest winning trade, % of gross profit"),
            col(&|s| pct(s.largest_win_pct_of_gross)),
        ),
        (tr("Largest losing trade"), col(&|s| f.money(s.largest_loss))),
        (
            tr("Largest losing trade, % of gross loss"),
            col(&|s| pct(s.largest_loss_pct_of_gross)),
        ),
        (tr("Avg # bars in trades"), col(&|s| num(s.avg_bars, 0))),
        (tr("Avg # bars in winning trades"), col(&|s| num(s.avg_bars_win, 0))),
        (tr("Avg # bars in losing trades"), col(&|s| num(s.avg_bars_loss, 0))),
    ];
    table(ui, t, "trades", &[tr("All"), tr("Long"), tr("Short")], &rows);
}

fn ratios(ui: &mut Ui, t: &Theme, r: &Report, f: &Fmt) {
    let v = |s: String| vec![(s, t.text)];
    let rows: Vec<(&str, Vec<(String, Color32)>)> = vec![
        (tr("Sharpe ratio"), v(num(r.sharpe, 3))),
        (tr("Sortino ratio"), v(num(r.sortino, 3))),
        (tr("Profit factor"), v(num(r.all.profit_factor, 3))),
        (tr("CAGR"), v(pct(r.cagr))),
        (tr("Return on initial capital"), v(pct(r.return_on_capital))),
        (tr("Account size required"), v(f.money(r.account_size_required))),
        (tr("Return on account size required"), v(pct(r.return_on_account_size))),
        (
            tr("Net profit as % of largest loss"),
            v(pct(r.net_profit_pct_of_largest_loss)),
        ),
        (tr("Return of max drawdown"), v(num(r.return_of_max_drawdown, 2))),
        (tr("Avg equity drawdown"), v(f.money(r.avg_drawdown))),
        (tr("Avg drawdown duration (bars)"), v(num(r.avg_drawdown_bars, 0))),
        (tr("Avg equity run-up"), v(f.money(r.avg_runup))),
        (tr("Avg run-up duration (bars)"), v(num(r.avg_runup_bars, 0))),
        (tr("Avg margin used"), v(f.money(r.avg_margin_used))),
        (tr("Max margin used"), v(f.money(r.max_margin_used))),
        (tr("Margin efficiency"), v(pct(r.margin_efficiency))),
        (tr("Margin calls"), v(r.margin_calls.to_string())),
    ];
    table(ui, t, "ratios", &[tr("Value")], &rows);
}

fn list_of_trades(ui: &mut Ui, t: &Theme, b: &Broker, f: &Fmt) {
    if b.trades.is_empty() {
        ui.label(egui::RichText::new(tr("No closed trades yet.")).color(t.text3));
        return;
    }
    let cols: [(&str, f32, bool); 10] = [
        (tr("Trade #"), 64.0, false),
        (tr("Type"), 92.0, false),
        (tr("Signal"), 190.0, false),
        (tr("Date/Time"), 136.0, false),
        (tr("Price"), 120.0, true),
        (tr("Contracts"), 110.0, true),
        (tr("Profit"), 190.0, true),
        (tr("Cum. profit"), 150.0, true),
        (tr("Run-up"), 130.0, true),
        (tr("Drawdown"), 130.0, true),
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
                    p.rect_filled(
                        r,
                        CornerRadius::same(4),
                        crate::motion::with_alpha(t.hover_wash(), crate::motion::hover(&resp)),
                    );
                    let side = if tr.long { "long" } else { "short" };
                    let pnl_c = tone(t, tr.pnl);
                    let lines: [[(String, Color32); 10]; 2] = [
                        [
                            (format!("{}", idx + 1), t.text),
                            (trf!("Exit {side}"), t.text2),
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
                            (trf!("Entry {side}"), t.text2),
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
        opentern_engine::QtyType::Fixed => trf!("{} contracts", qty_text(p.qty_value)),
        opentern_engine::QtyType::Cash => f.money(p.qty_value),
        opentern_engine::QtyType::PercentOfEquity => trf!("{}% of equity", num(p.qty_value, 2)),
    };
    let commission = match p.commission {
        opentern_engine::Commission::Percent => format!("{}%", num(p.commission_value, 4)),
        opentern_engine::Commission::CashPerContract => trf!("{} per contract", f.money(p.commission_value)),
        opentern_engine::Commission::CashPerOrder => trf!("{} per order", f.money(p.commission_value)),
    };
    let v = |s: String| vec![(s, t.text)];
    let rows: Vec<(&str, Vec<(String, Color32)>)> = vec![
        (tr("Symbol"), v(sym.to_string())),
        (tr("Timeframe"), v(tf.to_string())),
        (tr("Trading range"), v(range)),
        (tr("Bars"), v(bars.len().to_string())),
        (tr("Initial capital"), v(f.money(p.initial_capital))),
        (tr("Order size"), v(size)),
        (tr("Pyramiding"), v(format!("{} orders", p.pyramiding.max(1)))),
        (tr("Commission"), v(commission)),
        (tr("Slippage"), v(format!("{} ticks", p.slippage))),
        (
            tr("Margin for long positions"),
            v(format!("{}%", num(p.margin_long, 0))),
        ),
        (
            tr("Margin for short positions"),
            v(format!("{}%", num(p.margin_short, 0))),
        ),
        (
            tr("Fill orders on bar close"),
            v(if p.process_orders_on_close { tr("On") } else { tr("Off") }.into()),
        ),
    ];
    table(ui, t, "props", &[tr("Value")], &rows);
}

#[cfg(test)]
mod tests {
    use super::*;

    use egui::epaint::ClippedShape;

    fn overview_fixture() -> (Props, Vec<Bar>) {
        let props = Props {
            initial_capital: 1000.0,
            slippage: 15,
            mintick: 1.0,
            ..Props::default()
        };
        let bars = [
            (0, 10.0, 20.0),
            (1, 90.0, 100.0),
            (3, 50.0, 60.0),
            (7, 70.0, 80.0),
            (10, 60.0, 65.0),
        ]
        .into_iter()
        .map(|(minute, open, close)| Bar {
            time: minute * 60_000,
            open,
            high: close + 15.0,
            low: open,
            close,
            volume: 1.0,
        })
        .collect();
        (props, bars)
    }

    fn overview_entry(bar: usize) -> serde_json::Value {
        serde_json::json!({"bar": bar, "type": "entry", "id": "L", "qty": 2.0})
    }

    fn overview_test_context() -> egui::Context {
        let ctx = egui::Context::default();
        theme::install_fonts(&ctx, theme::FONTS[0].name);
        ctx
    }

    fn overview_frame(
        ctx: &egui::Context,
        broker: &Broker,
        bars: &[Bar],
        events: Vec<egui::Event>,
    ) -> Vec<ClippedShape> {
        overview_frame_at_height(ctx, broker, bars, 200.0, events)
    }

    fn overview_frame_at_height(
        ctx: &egui::Context,
        broker: &Broker,
        bars: &[Bar],
        height: f32,
        events: Vec<egui::Event>,
    ) -> Vec<ClippedShape> {
        let size = vec2(600.0, height);
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, size)),
            events,
            ..Default::default()
        };
        let theme = Theme::by_name("Tern Dark");
        let fmt = Fmt {
            ccy: "USD".into(),
            tz: jiff::tz::TimeZone::UTC,
        };
        let mut output = ctx.run_ui(input, |ui| equity_chart(ui, &theme, broker, bars, size, &fmt));
        output.textures_delta.clear();
        output.shapes
    }

    fn overview_hover(ctx: &egui::Context, broker: &Broker, bars: &[Bar], x: f32) -> Vec<ClippedShape> {
        overview_frame(ctx, broker, bars, vec![egui::Event::PointerMoved(pos2(x, 100.0))]);
        overview_frame(ctx, broker, bars, vec![])
    }

    fn overview_tooltip(shapes: &[ClippedShape]) -> Option<&str> {
        shapes.iter().find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.text().starts_with("1970-01-") => Some(text.galley.text()),
            _ => None,
        })
    }

    fn overview_curve(shapes: &[ClippedShape], width: f32) -> Option<&[egui::Pos2]> {
        shapes.iter().find_map(|shape| match &shape.shape {
            egui::Shape::Path(path) if path.stroke.width == width => Some(path.points.as_slice()),
            _ => None,
        })
    }

    fn overview_click(
        ctx: &egui::Context,
        broker: &Broker,
        bars: &[Bar],
        shapes: &[ClippedShape],
        name: &str,
    ) -> Vec<ClippedShape> {
        let pos = shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text() == name => Some(text.pos + vec2(4.0, 4.0)),
                _ => None,
            })
            .unwrap();
        let button = |pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        overview_frame(ctx, broker, bars, vec![egui::Event::PointerMoved(pos), button(true)]);
        overview_frame(ctx, broker, bars, vec![button(false)])
    }

    fn overview_context(broker: &Broker, bars: &[Bar], percent: bool) -> egui::Context {
        let ctx = overview_test_context();
        if percent {
            let shapes = overview_frame(&ctx, broker, bars, vec![]);
            overview_click(&ctx, broker, bars, &shapes, "Percent");
        }
        ctx
    }

    fn overview_axis_has(shapes: &[ClippedShape], label: &str) -> bool {
        shapes.iter().any(|shape| match &shape.shape {
            egui::Shape::Text(text) => text.pos.x >= 522.0 && text.galley.text() == label,
            _ => false,
        })
    }

    fn overview_drawdown_mesh<'a>(shapes: &'a [ClippedShape], theme: &Theme) -> Option<&'a egui::Mesh> {
        shapes.iter().find_map(|shape| match &shape.shape {
            egui::Shape::Mesh(mesh)
                if mesh
                    .vertices
                    .first()
                    .is_some_and(|v| v.color == theme.neg.gamma_multiply(0.55)) =>
            {
                Some(mesh.as_ref())
            }
            _ => None,
        })
    }

    #[test]
    fn overview_delayed_fill_aligns_series_and_hover() {
        let (props, bars) = overview_fixture();
        let broker = replay(props, &[overview_entry(1)], &bars, false);
        assert_eq!((broker.fills[0].bar, broker.fills[0].time), (2, bars[2].time));
        assert_eq!(broker.fills[0].price, 65.0);
        assert_eq!(broker.equity, [1000.0, 1000.0, 990.0, 1030.0, 1000.0]);

        for percent in [false, true] {
            let ctx = overview_context(&broker, &bars, percent);
            for (x, time, regular, percent_values) in [
                (
                    1.0,
                    "00:03",
                    ["−10.00 USD", "200.00 USD", "−10.00 USD"],
                    ["−1.00%", "20.00%", "−1.00%"],
                ),
                (
                    257.0,
                    "00:07",
                    ["30.00 USD", "600.00 USD", "0.00 USD"],
                    ["3.00%", "60.00%", "0.00%"],
                ),
                (
                    513.0,
                    "00:10",
                    ["0.00 USD", "300.00 USD", "−30.00 USD"],
                    ["0.00%", "30.00%", "−2.91%"],
                ),
            ] {
                let [equity, hold, dd] = if percent { percent_values } else { regular };
                let tooltip = format!("1970-01-01 {time}\nEquity  {equity}\nBuy & hold  {hold}\nDrawdown  {dd}");
                let shapes = overview_hover(&ctx, &broker, &bars, x);
                assert_eq!(overview_tooltip(&shapes), Some(tooltip.as_str()));
                for width in [1.6, 1.0] {
                    let curve = overview_curve(&shapes, width).unwrap();
                    assert_eq!(
                        curve.iter().map(|point| point.x).collect::<Vec<_>>(),
                        [0.0, 257.0, 514.0]
                    );
                }
            }
        }
    }

    #[test]
    fn overview_first_bar_fill_and_no_fill_keep_full_history() {
        let (mut props, bars) = overview_fixture();
        props.process_orders_on_close = true;
        let first = replay(props.clone(), &[overview_entry(0)], &bars, false);
        assert_eq!(first.fills[0].bar, 0);
        let mut limit = overview_entry(0);
        limit["limit"] = serde_json::json!(1.0);
        let none = replay(props, &[limit], &bars, false);
        assert!(none.fills.is_empty());
        assert_eq!(none.pending.len(), 1);

        for (broker, regular, percent_value) in [(&first, "−30.00 USD", "−3.00%"), (&none, "0.00 USD", "0.00%")] {
            for (percent, equity, hold) in [(false, regular, "1,000.00 USD"), (true, percent_value, "100.00%")] {
                let ctx = overview_context(broker, &bars, percent);
                let left = overview_hover(&ctx, broker, &bars, 1.0);
                let tooltip = overview_tooltip(&left).unwrap();
                assert!(tooltip.starts_with(&format!("1970-01-01 00:00\nEquity  {equity}\nBuy & hold  {hold}")));
                assert_eq!(overview_curve(&left, 1.6).unwrap().len(), bars.len());
                assert_eq!(overview_curve(&left, 1.0).unwrap().len(), bars.len());
                let right = overview_hover(&ctx, broker, &bars, 513.0);
                assert!(overview_tooltip(&right).unwrap().starts_with("1970-01-01 00:10"));
            }
        }
    }

    #[test]
    fn overview_last_bar_fill_draws_single_sample() {
        let (props, bars) = overview_fixture();
        let broker = replay(props, &[overview_entry(3)], &bars, false);
        assert_eq!(broker.fills[0].bar, 4);
        for (percent, tooltip) in [
            (
                false,
                "1970-01-01 00:10\nEquity  −20.00 USD\nBuy & hold  83.33 USD\nDrawdown  −20.00 USD",
            ),
            (
                true,
                "1970-01-01 00:10\nEquity  −2.00%\nBuy & hold  8.33%\nDrawdown  −2.00%",
            ),
        ] {
            let ctx = overview_context(&broker, &bars, percent);
            let shapes = overview_frame(&ctx, &broker, &bars, vec![]);
            let dots: Vec<_> = shapes
                .iter()
                .filter_map(|shape| match &shape.shape {
                    egui::Shape::Circle(circle) => Some(circle.center),
                    _ => None,
                })
                .collect();
            assert_eq!(dots.len(), 2);
            assert!(dots.iter().all(|dot| dot.x == 0.0 && dot.y.is_finite()));
            let shapes = overview_hover(&ctx, &broker, &bars, 513.0);
            assert_eq!(overview_tooltip(&shapes), Some(tooltip));
        }
    }

    #[test]
    fn overview_toggles_and_autoscale_use_visible_series_after_fill() {
        for percent in [false, true] {
            check_overview_toggles_and_autoscale(percent);
        }
    }

    fn check_overview_toggles_and_autoscale(percent: bool) {
        let (props, bars) = overview_fixture();
        let broker = replay(props, &[overview_entry(1)], &bars, false);
        let theme = Theme::by_name("Tern Dark");
        let ctx = overview_context(&broker, &bars, percent);
        let names = ["Equity", "Buy & hold", "Drawdown"];
        let mut shapes = overview_frame(&ctx, &broker, &bars, vec![]);
        let plot_height = 200.0 - 26.0;
        let dd_top = 200.0 - plot_height * 0.22;
        // Each transition is a single click, covering every legend combination and restoring the default.
        for (toggle, visible) in [
            (None, [true, true, true]),
            (Some(1), [true, false, true]),
            (Some(2), [true, false, false]),
            (Some(1), [true, true, false]),
            (Some(0), [false, true, false]),
            (Some(2), [false, true, true]),
            (Some(1), [false, false, true]),
            (Some(2), [false, false, false]),
            (Some(1), [false, true, false]),
            (Some(0), [true, true, false]),
            (Some(2), [true, true, true]),
        ] {
            if let Some(i) = toggle {
                shapes = overview_click(&ctx, &broker, &bars, &shapes, names[i]);
                assert_eq!(
                    overview_curve(&shapes, 1.6).is_some(),
                    visible[0],
                    "click frame {visible:?}"
                );
                assert_eq!(
                    overview_curve(&shapes, 1.0).is_some(),
                    visible[1],
                    "click frame {visible:?}"
                );
            }

            shapes = overview_hover(&ctx, &broker, &bars, 257.0);
            let tooltip = overview_tooltip(&shapes);
            assert_eq!(tooltip.is_some(), visible.contains(&true), "{visible:?}");
            if let Some(tooltip) = tooltip {
                assert!(tooltip.starts_with("1970-01-01 00:07"));
                for (name, on) in names.into_iter().zip(visible) {
                    assert_eq!(tooltip.contains(&format!("\n{name}  ")), on, "{visible:?}");
                }
                assert!(
                    tooltip
                        .lines()
                        .skip(1)
                        .all(|line| line.ends_with(if percent { "%" } else { " USD" }))
                );
            }
            let curve_height = if visible[2] { dd_top - 6.0 - 26.0 } else { plot_height };
            for (i, width) in [1.6, 1.0].into_iter().enumerate() {
                let curve = overview_curve(&shapes, width);
                assert_eq!(curve.is_some(), visible[i], "{visible:?}");
                if let Some(curve) = curve {
                    let height = (curve[0].y - curve[1].y).abs();
                    let fraction = if visible[0] && visible[1] {
                        [40.0, 400.0][i] / 610.0
                    } else if i == 1 {
                        400.0 / 600.0
                    } else {
                        1.0
                    };
                    assert!(
                        (height - curve_height * fraction / 1.12).abs() < 0.01,
                        "{visible:?} {height}"
                    );
                }
            }
            let drawdown_mesh = overview_drawdown_mesh(&shapes, &theme);
            assert_eq!(drawdown_mesh.is_some(), visible[2], "{visible:?}");
            if let Some(mesh) = drawdown_mesh {
                let top = if visible[0] || visible[1] { dd_top } else { 26.0 };
                assert_eq!(mesh.vertices.len(), 8);
                assert_eq!(mesh.vertices[0].pos.y, top);
                assert_eq!(mesh.vertices[4].pos.y, top);
                assert_eq!(mesh.vertices[6].pos.y, 200.0);
            }
        }
    }

    #[test]
    fn overview_localizes_partial_tooltip_and_keeps_visibility_across_languages() {
        use crate::i18n::{self, Lang};

        let (props, bars) = overview_fixture();
        let broker = replay(props, &[overview_entry(1)], &bars, false);
        let ctx = overview_test_context();
        i18n::set(Lang::German);
        let shapes = overview_frame(&ctx, &broker, &bars, vec![]);
        overview_click(&ctx, &broker, &bars, &shapes, "Buy & Hold");
        let shapes = overview_hover(&ctx, &broker, &bars, 257.0);
        let tooltip = overview_tooltip(&shapes).unwrap();
        assert!(tooltip.contains("\nKapital  "));
        assert!(!tooltip.contains("\nBuy & Hold  "));

        i18n::set(Lang::English);
        let shapes = overview_hover(&ctx, &broker, &bars, 257.0);
        let tooltip = overview_tooltip(&shapes).unwrap();
        assert!(tooltip.contains("\nEquity  "));
        assert!(!tooltip.contains("\nBuy & hold  "));
    }

    #[test]
    fn overview_scale_switch_updates_axis_and_drawdown_on_the_click_frame() {
        let (props, bars) = overview_fixture();
        let broker = replay(props, &[overview_entry(1)], &bars, false);
        let ctx = overview_test_context();
        let theme = Theme::by_name("Tern Dark");
        let shapes = overview_frame(&ctx, &broker, &bars, vec![]);
        let shapes = overview_click(&ctx, &broker, &bars, &shapes, "Buy & hold");

        for (name, bottom, fraction, tooltip) in [
            ("Regular", "−30", 1.0 / 3.0, "Equity  −10.00 USD\nDrawdown  −10.00 USD"),
            ("Percent", "−2.91%", 1.03 / 3.0, "Equity  −1.00%\nDrawdown  −1.00%"),
            ("Regular", "−30", 1.0 / 3.0, "Equity  −10.00 USD\nDrawdown  −10.00 USD"),
        ] {
            let frame = overview_click(&ctx, &broker, &bars, &shapes, name);
            assert!(overview_axis_has(&frame, bottom), "{name}");
            assert!(overview_curve(&frame, 1.0).is_none(), "{name}");
            let mesh = overview_drawdown_mesh(&frame, &theme).unwrap();
            let first = mesh.vertices[2].pos.y - mesh.vertices[0].pos.y;
            let last = mesh.vertices[6].pos.y - mesh.vertices[4].pos.y;
            assert!((first / last - fraction).abs() < 1e-4, "{name}");

            let hover = overview_hover(&ctx, &broker, &bars, 1.0);
            assert!(overview_tooltip(&hover).unwrap().ends_with(tooltip), "{name}");
        }

        overview_click(&ctx, &broker, &bars, &shapes, "Percent");
        let fresh = overview_test_context();
        assert!(
            overview_tooltip(&overview_hover(&fresh, &broker, &bars, 1.0))
                .unwrap()
                .ends_with("Drawdown  −10.00 USD")
        );
        assert!(
            overview_tooltip(&overview_hover(&ctx, &broker, &bars, 1.0))
                .unwrap()
                .ends_with("Drawdown  −1.00%")
        );
    }

    #[test]
    fn overview_zero_lines_and_equity_fill_use_the_performance_baseline() {
        let (props, bars) = overview_fixture();
        let broker = replay(props, &[overview_entry(1)], &bars, false);
        let theme = Theme::by_name("Tern Dark");
        for (percent, label) in [(false, "0"), (true, "0.00%")] {
            let ctx = overview_context(&broker, &bars, percent);
            let shapes = overview_frame(&ctx, &broker, &bars, vec![]);
            let zeroes: Vec<_> = shapes
                .iter()
                .filter_map(|shape| match &shape.shape {
                    egui::Shape::LineSegment { points, stroke } if *stroke == Stroke::new(1.5, theme.text3) => {
                        assert_eq!(points[0].y, points[1].y);
                        Some(points[0].y)
                    }
                    _ => None,
                })
                .collect();
            let equity = overview_curve(&shapes, 1.6).unwrap();
            let drawdown = overview_drawdown_mesh(&shapes, &theme).unwrap();
            assert_eq!(zeroes, [equity[2].y, drawdown.vertices[0].pos.y]);
            assert!(zeroes[0] < zeroes[1]);
            assert!(overview_axis_has(&shapes, label));

            let colors = [theme.neg.gamma_multiply(0.12), theme.pos.gamma_multiply(0.12)];
            let fill = shapes.iter().find_map(|shape| match &shape.shape {
                egui::Shape::Mesh(mesh) if mesh.vertices.first().is_some_and(|v| v.color == colors[0]) => Some(mesh),
                _ => None,
            });
            let fill = fill.unwrap();
            let crossing = pos2(64.25, zeroes[0]);
            for color in colors {
                assert!(
                    fill.vertices
                        .iter()
                        .any(|v| v.pos.distance(crossing) < 0.01 && v.color == color)
                );
            }

            let shapes = overview_click(&ctx, &broker, &bars, &shapes, "Equity");
            let hold_only = overview_click(&ctx, &broker, &bars, &shapes, "Drawdown");
            let zeroes: Vec<_> = hold_only
                .iter()
                .filter_map(|shape| match &shape.shape {
                    egui::Shape::LineSegment { points, stroke } if *stroke == Stroke::new(1.5, theme.text3) => {
                        assert!(points.iter().all(|&point| shape.clip_rect.contains(point)));
                        Some(*points)
                    }
                    _ => None,
                })
                .collect();
            let [[left, right]] = zeroes.as_slice() else {
                panic!("Expected a single zero line with only Buy & hold visible");
            };
            assert_eq!((left.x, right.x), (0.0, 514.0));
            assert_eq!(left.y, right.y);
            assert!(left.y > 26.0 && left.y < 200.0);
            let hold = overview_curve(&hold_only, 1.0).unwrap();
            assert!(hold.iter().all(|point| point.y < left.y));
            assert!(overview_axis_has(&hold_only, label));
        }
    }

    #[test]
    fn overview_full_tooltip_fits_the_minimum_plot_height() {
        let (props, bars) = overview_fixture();
        let broker = replay(props, &[overview_entry(1)], &bars, false);
        let plot = Rect::from_min_max(pos2(0.0, 26.0), pos2(514.0, 90.0));
        for (percent, unit) in [(false, " USD"), (true, "%")] {
            let ctx = overview_context(&broker, &bars, percent);
            let pointer = egui::Event::PointerMoved(pos2(257.0, 55.0));
            overview_frame_at_height(&ctx, &broker, &bars, 90.0, vec![pointer]);
            let shapes = overview_frame_at_height(&ctx, &broker, &bars, 90.0, vec![]);
            let (background, background_clip, text, text_clip) = shapes
                .windows(2)
                .find_map(|pair| match (&pair[0].shape, &pair[1].shape) {
                    (egui::Shape::Rect(rect), egui::Shape::Text(text))
                        if text.galley.text().starts_with("1970-01-01 00:07") =>
                    {
                        Some((rect.rect, pair[0].clip_rect, text, pair[1].clip_rect))
                    }
                    _ => None,
                })
                .expect("Tooltip background and text");
            assert_eq!(background.left(), 269.0);
            assert!(plot.contains_rect(background), "{unit} {background:?}");
            assert!(background_clip.contains_rect(background), "{unit} {background:?}");
            assert_eq!(text.galley.rows.len(), 4);
            let mut lines = text.galley.text().lines();
            assert_eq!(lines.next(), Some("1970-01-01 00:07"));
            for (line, label) in lines.zip(["Equity  ", "Buy & hold  ", "Drawdown  "]) {
                assert!(line.starts_with(label) && line.ends_with(unit), "{line}");
            }
            for row in &text.galley.rows {
                let bounds = row.rect().translate(text.pos.to_vec2());
                assert!(background.contains_rect(bounds), "{unit} {bounds:?}");
                assert!(text_clip.contains_rect(bounds), "{unit} {bounds:?}");
            }
        }
    }

    #[test]
    fn overview_buy_and_hold_falls_back_to_zero_performance_for_nonpositive_open() {
        let (props, mut bars) = overview_fixture();
        let broker = replay(props, &[overview_entry(1)], &bars, false);
        for open in [0.0, -1.0] {
            bars[broker.fills[0].bar].open = open;
            for (percent, label) in [(false, "0.00 USD"), (true, "0.00%")] {
                let ctx = overview_context(&broker, &bars, percent);
                for x in [1.0, 513.0] {
                    let shapes = overview_hover(&ctx, &broker, &bars, x);
                    assert!(
                        overview_tooltip(&shapes)
                            .unwrap()
                            .contains(&format!("\nBuy & hold  {label}\n"))
                    );
                    let hold = overview_curve(&shapes, 1.0).unwrap();
                    assert!(hold.iter().all(|p| p.y.is_finite() && p.y == hold[0].y));
                }
            }
        }
    }

    #[test]
    fn overview_nonpositive_initial_capital_disables_percent() {
        let (props, bars) = overview_fixture();
        let funded = replay(props, &[], &bars, false);
        for cap in [0.0, -1.0] {
            let props = Props::from_json(&serde_json::json!({"initial_capital": cap}), 0.01, 1.0);
            let broker = replay(props, &[], &bars, false);
            assert_eq!(broker.props.initial_capital, cap);

            let ctx = overview_test_context();
            let shapes = overview_frame(&ctx, &broker, &bars, vec![]);
            let click = overview_click(&ctx, &broker, &bars, &shapes, "Percent");
            assert!(overview_axis_has(&click, "0"));
            assert!(!overview_axis_has(&click, "0.00%"));
            let hover = overview_hover(&ctx, &funded, &bars, 1.0);
            assert!(overview_tooltip(&hover).unwrap().contains("Equity  0.00 USD"));

            let ctx = overview_context(&funded, &bars, true);
            let hover = overview_hover(&ctx, &broker, &bars, 1.0);
            assert!(overview_tooltip(&hover).unwrap().contains("Equity  0.00 USD"));
            let hover = overview_hover(&ctx, &funded, &bars, 1.0);
            assert!(overview_tooltip(&hover).unwrap().contains("Equity  0.00%"));
        }
    }

    #[test]
    fn overview_percent_drawdown_skips_nonpositive_pine_peaks_and_resumes() {
        let bars = [(0, 2.0), (1, 1.5), (2, 3.5), (3, 3.25)].map(|(day, v)| Bar {
            time: day * 86_400_000,
            open: v,
            high: v,
            low: v,
            close: v,
            volume: 1.0,
        });
        let theme = Theme::by_name("Tern Dark");
        for (slippage, equity, percent_equity, regular_equity, resumed_dd) in [
            (100, [0.0, -0.5, 1.5, 1.25], "−150.00%", "−1.50 USD", "−16.67%"),
            (150, [-0.5, -1.0, 1.0, 0.75], "−200.00%", "−2.00 USD", "−25.00%"),
        ] {
            // Run the real Pine strategy that fills on its first close.
            let script = opentern_pine::compile(
                &format!(
                    r#"//@version=6
strategy("Overview drawdown", initial_capital=1, margin_long=0, process_orders_on_close=true, slippage={slippage})
if bar_index == 0
    strategy.entry("L", strategy.long, qty=1)
"#
                ),
                "overview_drawdown.pine",
                opentern_pine::Env::default(),
            )
            .unwrap();
            let mut run = script.study(&opentern_series::study::Inputs::default());
            run.run(&bars, Timeframe::new(1, opentern_core::Unit::Day));
            assert!(run.error().is_none(), "{:?}", run.error());
            let broker = run.broker().unwrap();
            assert_eq!(broker.props.initial_capital, 1.0);
            assert_eq!(broker.fills[0].bar, 0);
            assert_eq!(broker.equity, equity);

            let ctx = overview_context(broker, &bars, true);
            let unavailable = overview_hover(&ctx, broker, &bars[..2], 513.0);
            let tooltip = overview_tooltip(&unavailable).unwrap();
            assert!(tooltip.ends_with(&format!("Equity  {percent_equity}\nBuy & hold  −25.00%\nDrawdown  —")));
            assert!(overview_drawdown_mesh(&unavailable, &theme).is_none());
            assert!(overview_axis_has(&unavailable, "0.00%"));
            for width in [1.6, 1.0] {
                assert!(
                    overview_curve(&unavailable, width)
                        .unwrap()
                        .iter()
                        .all(|p| p.is_finite())
                );
            }

            for (x, value) in [(1.0, "—"), (171.0, "—"), (343.0, "0.00%"), (513.0, resumed_dd)] {
                let shapes = overview_hover(&ctx, broker, &bars, x);
                let tooltip = overview_tooltip(&shapes).unwrap();
                assert!(tooltip.ends_with(&format!("Drawdown  {value}")), "{tooltip}");
                let dd = overview_drawdown_mesh(&shapes, &theme).unwrap();
                assert_eq!(dd.vertices.len(), 4);
                assert!(dd.vertices.iter().all(|v| v.pos.is_finite() && v.pos.x > 400.0));
                assert_eq!(dd.vertices[2].pos.y, 200.0);
                assert!(overview_axis_has(&shapes, resumed_dd));
            }

            let shapes = overview_frame(&ctx, broker, &bars, vec![]);
            let shapes = overview_click(&ctx, broker, &bars, &shapes, "Regular");
            assert_eq!(overview_drawdown_mesh(&shapes, &theme).unwrap().vertices.len(), 8);
            let shapes = overview_hover(&ctx, broker, &bars[..2], 513.0);
            assert!(overview_tooltip(&shapes).unwrap().ends_with(&format!(
                "Equity  {regular_equity}\nBuy & hold  −0.25 USD\nDrawdown  −0.50 USD"
            )));
            for (x, value) in [(1.0, "0.00"), (171.0, "−0.50"), (343.0, "0.00"), (513.0, "−0.25")] {
                let shapes = overview_hover(&ctx, broker, &bars, x);
                let tooltip = overview_tooltip(&shapes).unwrap();
                assert!(tooltip.ends_with(&format!("Drawdown  {value} USD")), "{tooltip}");
            }
        }
    }

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
        let zero = vec![serde_json::json!({"bar": 0, "type": "entry", "id": "L", "long": true, "qty": 0.0})];
        assert!(replay(Props::default(), &zero, &bars, false).lots.is_empty());
        let b = replay(Props::default(), &intents, &bars, false);
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

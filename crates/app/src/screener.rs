//! The Screener tab: a list of symbols (a watchlist or a preset), each
//! fetched once and measured (last, change, performance, volume, RSI,
//! distance from moving averages, ATR), optionally with a script's value
//! (a built-in or a Pine script run on every symbol, on a thread pool, in
//! the AmiBroker "exploration" spirit). Filters keep the rows that match;
//! clicking a row opens it on the chart.

use crate::app::App;
use crate::icons::ph;
use crate::theme::{self, Theme};
use egui::{Align, Layout, Sense, Ui, vec2};
use opentern_core::{AssetKind, Bar, Symbol, Timeframe};
use opentern_data::Request;
use opentern_series::Instance;
use opentern_series::study::{Inputs, Value, sessions};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Bars fetched per symbol (enough for a 200 SMA with room to warm up).
const BARS: usize = 400;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Field {
    Last,
    Change,
    PerfWeek,
    PerfMonth,
    Volume,
    RelVolume,
    Rsi,
    Sma50,
    Sma200,
    Atr,
    Script,
}

impl Field {
    pub const ALL: [Field; 11] = [
        Field::Last,
        Field::Change,
        Field::PerfWeek,
        Field::PerfMonth,
        Field::Volume,
        Field::RelVolume,
        Field::Rsi,
        Field::Sma50,
        Field::Sma200,
        Field::Atr,
        Field::Script,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Field::Last => "Price",
            Field::Change => "Change %",
            Field::PerfWeek => "Perf 1W %",
            Field::PerfMonth => "Perf 1M %",
            Field::Volume => "Volume",
            Field::RelVolume => "Rel volume",
            Field::Rsi => "RSI (14)",
            Field::Sma50 => "vs SMA 50 %",
            Field::Sma200 => "vs SMA 200 %",
            Field::Atr => "ATR %",
            Field::Script => "Script",
        }
    }

    fn pct(self) -> bool {
        matches!(
            self,
            Field::Change | Field::PerfWeek | Field::PerfMonth | Field::Sma50 | Field::Sma200 | Field::Atr
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Op {
    Above,
    Below,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Filter {
    pub field: Field,
    pub op: Op,
    pub value: f64,
}

/// What a script column runs.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub enum ScreenScript {
    #[default]
    None,
    /// A built-in by id, and which plot to read.
    Builtin(String, usize),
    /// A Pine file in the scripts folder, and which plot.
    Pine(String, usize),
}

/// Where the symbols come from.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Universe {
    Watchlist(i64),
    Preset(String),
}

/// Saved between launches (settings key `screener`).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct ScreenSetup {
    pub universe: Universe,
    pub tf: String,
    pub filters: Vec<Filter>,
    pub script: ScreenScript,
}

impl Default for ScreenSetup {
    fn default() -> Self {
        ScreenSetup {
            universe: Universe::Preset("Crypto majors".into()),
            tf: "1D".into(),
            filters: Vec::new(),
            script: ScreenScript::None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Row {
    pub symbol: Symbol,
    pub values: HashMap<Field, f64>,
    pub error: Option<String>,
    pub done: bool,
}

impl Row {
    pub fn get(&self, f: Field) -> f64 {
        self.values.get(&f).copied().unwrap_or(f64::NAN)
    }

    fn passes(&self, filters: &[Filter]) -> bool {
        filters.iter().all(|f| {
            let v = self.get(f.field);
            v.is_finite()
                && match f.op {
                    Op::Above => v > f.value,
                    Op::Below => v < f.value,
                }
        })
    }
}

/// A computed row coming back from the thread pool.
struct Computed {
    run: u64,
    id: String,
    values: HashMap<Field, f64>,
    error: Option<String>,
}

pub struct Screener {
    pub setup: ScreenSetup,
    pub rows: Vec<Row>,
    /// The run whose answers are wanted (older ones are dropped).
    run: u64,
    seq_base: u64,
    pending: usize,
    sort: (Field, bool),
    tx: crossbeam_channel::Sender<Computed>,
    rx: crossbeam_channel::Receiver<Computed>,
    pub status: String,
    loaded: bool,
}

impl Default for Screener {
    fn default() -> Self {
        let (tx, rx) = crossbeam_channel::unbounded();
        Screener {
            setup: ScreenSetup::default(),
            rows: Vec::new(),
            run: 0,
            seq_base: 1 << 40,
            pending: 0,
            sort: (Field::Change, true),
            tx,
            rx,
            status: String::new(),
            loaded: false,
        }
    }
}

fn sym(id: &str, kind: AssetKind) -> Symbol {
    let ex = id.split(':').next().unwrap_or("");
    Symbol {
        id: id.into(),
        description: String::new(),
        kind,
        currency: match ex {
            "NSE" | "BSE" => "INR",
            "BINANCE" => "USDT",
            _ => "USD",
        }
        .into(),
        tick_size: if ex == "NSE" { 0.05 } else { 0.01 },
        min_qty: 0.0,
        timezone: match ex {
            "NSE" | "BSE" => "Asia/Kolkata",
            "NASDAQ" | "NYSE" | "AMEX" => "America/New_York",
            _ => "UTC",
        }
        .into(),
    }
}

/// Built-in lists of liquid public symbols, for screening without making
/// a watchlist first.
pub fn presets() -> Vec<(&'static str, Vec<Symbol>)> {
    let crypto = [
        "BTC", "ETH", "SOL", "BNB", "XRP", "ADA", "DOGE", "AVAX", "LINK", "DOT", "TRX", "LTC", "BCH", "NEAR", "ATOM",
        "UNI", "ETC", "APT", "ARB", "OP", "SUI", "FIL", "INJ", "AAVE",
    ]
    .iter()
    .map(|t| sym(&format!("BINANCE:{t}USDT"), AssetKind::Crypto))
    .collect();
    let us = [
        "NASDAQ:AAPL",
        "NASDAQ:MSFT",
        "NASDAQ:NVDA",
        "NASDAQ:AMZN",
        "NASDAQ:GOOGL",
        "NASDAQ:META",
        "NASDAQ:TSLA",
        "NASDAQ:AVGO",
        "NASDAQ:NFLX",
        "NASDAQ:AMD",
        "NASDAQ:COST",
        "NYSE:JPM",
        "NYSE:V",
        "NYSE:UNH",
        "NYSE:XOM",
        "NYSE:LLY",
        "NYSE:MA",
        "NYSE:HD",
        "NYSE:PG",
        "NYSE:WMT",
    ]
    .iter()
    .map(|id| sym(id, AssetKind::Stock))
    .collect();
    let india = [
        "RELIANCE",
        "TCS",
        "HDFCBANK",
        "ICICIBANK",
        "INFY",
        "BHARTIARTL",
        "ITC",
        "SBIN",
        "LT",
        "HINDUNILVR",
        "KOTAKBANK",
        "AXISBANK",
        "BAJFINANCE",
        "MARUTI",
        "SUNPHARMA",
        "HCLTECH",
        "TITAN",
        "ASIANPAINT",
        "WIPRO",
        "ULTRACEMCO",
    ]
    .iter()
    .map(|t| sym(&format!("NSE:{t}"), AssetKind::Stock))
    .collect();
    vec![
        ("Crypto majors", crypto),
        ("US large caps", us),
        ("India large caps", india),
    ]
}

/// What runs on each symbol's bars, prepared on the UI thread.
#[derive(Clone)]
enum Job {
    None,
    Builtin(&'static opentern_series::Meta, Inputs, usize),
    Pine(String, String, opentern_pine::Env, usize),
}

fn last_of(
    meta: &'static opentern_series::Meta,
    inputs: &Inputs,
    bars: &[Bar],
    sess: &[bool],
    tf: Timeframe,
    plot: usize,
) -> f64 {
    let mut inst = Instance::new(meta, inputs);
    inst.recompute(bars, sess, tf);
    inst.out
        .plots
        .get(plot)
        .and_then(|p| p.last().copied())
        .unwrap_or(f64::NAN)
}

fn builtin_last(id: &str, len: Option<i64>, bars: &[Bar], sess: &[bool], tf: Timeframe) -> f64 {
    let Some(m) = opentern_series::builtins::by_id(id) else {
        return f64::NAN;
    };
    let mut ins = m.default_inputs();
    if let Some(l) = len {
        ins.set("length", Value::Int(l));
    }
    last_of(m, &ins, bars, sess, tf, 0)
}

/// The close at or before `t`.
fn close_at(bars: &[Bar], t: i64) -> Option<f64> {
    let i = bars.partition_point(|b| b.time <= t);
    (i > 0).then(|| bars[i - 1].close)
}

/// Every column for one symbol.
fn measure(bars: &[Bar], tf: Timeframe, tz: &jiff::tz::TimeZone, job: &Job) -> Result<HashMap<Field, f64>, String> {
    let n = bars.len();
    let last = bars.last().ok_or("No bars came back.")?;
    let sess = sessions(bars, tf, tz);
    let mut v = HashMap::new();
    v.insert(Field::Last, last.close);
    if n >= 2 && bars[n - 2].close != 0.0 {
        v.insert(Field::Change, (last.close / bars[n - 2].close - 1.0) * 100.0);
    }
    let day = 86_400_000;
    for (f, days) in [(Field::PerfWeek, 7), (Field::PerfMonth, 30)] {
        if let Some(c) = close_at(bars, last.time - days * day).filter(|c| *c != 0.0) {
            v.insert(f, (last.close / c - 1.0) * 100.0);
        }
    }
    v.insert(Field::Volume, last.volume);
    if n > 21 {
        let avg = bars[n - 21..n - 1].iter().map(|b| b.volume).sum::<f64>() / 20.0;
        if avg > 0.0 {
            v.insert(Field::RelVolume, last.volume / avg);
        }
    }
    v.insert(Field::Rsi, builtin_last("rsi", None, bars, &sess, tf));
    for (f, len) in [(Field::Sma50, 50), (Field::Sma200, 200)] {
        let sma = builtin_last("sma", Some(len), bars, &sess, tf);
        if sma.is_finite() && sma != 0.0 {
            v.insert(f, (last.close / sma - 1.0) * 100.0);
        }
    }
    let atr = builtin_last("atr", None, bars, &sess, tf);
    if atr.is_finite() && last.close != 0.0 {
        v.insert(Field::Atr, atr / last.close * 100.0);
    }
    match job {
        Job::None => {}
        Job::Builtin(m, ins, plot) => {
            v.insert(Field::Script, last_of(m, ins, bars, &sess, tf, *plot));
        }
        Job::Pine(src, file, env, plot) => {
            let script = opentern_pine::compile(src, file, env.clone()).map_err(|e| e.to_string())?;
            let meta = script.meta();
            let inst_val = {
                let mut inst = Instance::with_factory(meta, &meta.default_inputs(), script.factory());
                inst.recompute(bars, &sess, tf);
                inst.out
                    .plots
                    .get(*plot)
                    .and_then(|p| p.last().copied())
                    .unwrap_or(f64::NAN)
            };
            v.insert(Field::Script, inst_val);
        }
    }
    Ok(v)
}

impl App {
    fn screener_symbols(&self) -> Vec<Symbol> {
        match &self.screener.setup.universe {
            Universe::Watchlist(id) => self
                .watchlists
                .iter()
                .find(|w| w.id == *id)
                .map(|w| w.symbols.clone())
                .unwrap_or_default(),
            Universe::Preset(name) => presets()
                .into_iter()
                .find(|(n, _)| n == name)
                .map(|(_, s)| s)
                .unwrap_or_default(),
        }
    }

    fn universe_name(&self) -> String {
        match &self.screener.setup.universe {
            Universe::Watchlist(id) => self
                .watchlists
                .iter()
                .find(|w| w.id == *id)
                .map(|w| w.name.clone())
                .unwrap_or_else(|| "Watchlist".into()),
            Universe::Preset(n) => n.clone(),
        }
    }

    /// Starts a run: every symbol's bars are fetched; each is measured on
    /// the thread pool when it arrives.
    pub fn run_screener(&mut self) {
        let symbols = self.screener_symbols();
        let Some(tf) = Timeframe::parse(&self.screener.setup.tf) else {
            return;
        };
        let sc = &mut self.screener;
        sc.run += 1;
        sc.seq_base = (1 << 40) + sc.run * 10_000;
        sc.pending = symbols.len();
        sc.rows = symbols
            .iter()
            .map(|s| Row {
                symbol: s.clone(),
                values: HashMap::new(),
                error: None,
                done: false,
            })
            .collect();
        sc.status = format!("Loading {} symbols…", symbols.len());
        for (k, s) in symbols.into_iter().enumerate() {
            self.hub.send(Request::Fetch {
                seq: self.screener.seq_base + k as u64,
                symbol: s,
                tf,
                count: BARS,
            });
        }
        self.save_screener();
    }

    fn save_screener(&mut self) {
        if let Ok(j) = serde_json::to_string(&self.screener.setup) {
            let _ = self.store.set_value("screener", &j);
        }
    }

    /// Bars for the screener arrived: measure them off the UI thread.
    pub(crate) fn screener_fetched(&mut self, seq: u64, id: &str, result: Result<Vec<Bar>, String>) {
        let sc = &self.screener;
        if seq < sc.seq_base || seq >= sc.seq_base + 10_000 {
            return;
        }
        let Some(row) = sc.rows.iter().find(|r| r.symbol.id == id) else {
            return;
        };
        let symbol = row.symbol.clone();
        let tf = Timeframe::parse(&sc.setup.tf).unwrap_or(Timeframe::parse("1D").expect("valid"));
        let job = self.screen_job(&symbol);
        let (tx, run) = (sc.tx.clone(), sc.run);
        let ctx = self.egui_ctx.clone();
        rayon::spawn(move || {
            let tz = jiff::tz::TimeZone::get(&symbol.timezone).unwrap_or(jiff::tz::TimeZone::UTC);
            let (values, error) = match result.and_then(|bars| {
                std::panic::catch_unwind(|| measure(&bars, tf, &tz, &job))
                    .unwrap_or_else(|_| Err("The script crashed.".into()))
            }) {
                Ok(v) => (v, None),
                Err(e) => (HashMap::new(), Some(e)),
            };
            let _ = tx.send(Computed {
                run,
                id: symbol.id.clone(),
                values,
                error,
            });
            ctx.request_repaint();
        });
    }

    fn screen_job(&self, symbol: &Symbol) -> Job {
        match &self.screener.setup.script {
            ScreenScript::None => Job::None,
            ScreenScript::Builtin(id, plot) => match opentern_series::builtins::by_id(id) {
                Some(m) => {
                    let (ins, _) = self.study_defaults(m);
                    Job::Builtin(m, ins, *plot)
                }
                None => Job::None,
            },
            ScreenScript::Pine(file, plot) => match std::fs::read_to_string(self.scripts_dir().join(file)) {
                Ok(src) => Job::Pine(src, file.clone(), self.pine_env_for(symbol), *plot),
                Err(_) => Job::None,
            },
        }
    }

    /// Results from the pool.
    pub(crate) fn poll_screener(&mut self) {
        if !self.screener.loaded {
            self.screener.loaded = true;
            if let Some(s) = self
                .store
                .value("screener")
                .and_then(|j| serde_json::from_str::<ScreenSetup>(&j).ok())
            {
                self.screener.setup = s;
            }
        }
        let sc = &mut self.screener;
        let mut got = false;
        while let Ok(c) = sc.rx.try_recv() {
            if c.run != sc.run {
                continue;
            }
            got = true;
            if let Some(r) = sc.rows.iter_mut().find(|r| r.symbol.id == c.id && !r.done) {
                r.values = c.values;
                r.error = c.error;
                r.done = true;
                sc.pending = sc.pending.saturating_sub(1);
            }
        }
        if got {
            let done = sc.rows.iter().filter(|r| r.done).count();
            let errors = sc.rows.iter().filter(|r| r.error.is_some()).count();
            sc.status = if sc.pending > 0 {
                format!("{done} of {} done…", sc.rows.len())
            } else if errors > 0 {
                format!("Done. {errors} couldn't load (hover them to see why).")
            } else {
                "Done.".into()
            };
        }
    }

    pub(crate) fn screener_tab(&mut self, ui: &mut Ui, t: &Theme) {
        // ---- toolbar
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let current = self.universe_name();
            crate::widgets::combo("screen-universe")
                .selected_text(current)
                .width(170.0)
                .show_ui(ui, |ui| {
                    for (name, _) in presets() {
                        let u = Universe::Preset(name.into());
                        if crate::widgets::option(ui, self.screener.setup.universe == u, name).clicked() {
                            self.screener.setup.universe = u;
                        }
                    }
                    ui.separator();
                    for wl in self.watchlists.clone() {
                        let u = Universe::Watchlist(wl.id);
                        if crate::widgets::option(
                            ui,
                            self.screener.setup.universe == u,
                            format!("{}  {}", ph::LIST_BULLETS, wl.name),
                        )
                        .clicked()
                        {
                            self.screener.setup.universe = u;
                        }
                    }
                });
            crate::widgets::combo("screen-tf")
                .selected_text(self.screener.setup.tf.clone())
                .width(60.0)
                .show_ui(ui, |ui| {
                    for tf in ["15m", "1h", "4h", "1D", "1W"] {
                        let on = self.screener.setup.tf == tf;
                        if crate::widgets::option(ui, on, tf).clicked() {
                            self.screener.setup.tf = tf.into();
                        }
                    }
                });
            self.script_picker(ui, t);
            if crate::widgets::button(ui, crate::widgets::Kind::Primary, true, Some(ph::PLAY), "Run")
                .on_hover_text("Fetch and measure every symbol")
                .clicked()
            {
                self.run_screener();
            }
            ui.label(egui::RichText::new(&self.screener.status).color(t.text3));
        });
        ui.add_space(4.0);
        // ---- filters
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let mut remove = None;
            for (k, f) in self.screener.setup.filters.iter_mut().enumerate() {
                egui::Frame::new()
                    .fill(t.elevated)
                    .corner_radius(6)
                    .inner_margin(egui::Margin::symmetric(6, 2))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            crate::widgets::combo(("flt-field", k))
                                .selected_text(f.field.name())
                                .width(110.0)
                                .show_ui(ui, |ui| {
                                    for fld in Field::ALL {
                                        crate::widgets::option_value(ui, &mut f.field, fld, fld.name());
                                    }
                                });
                            crate::widgets::combo(("flt-op", k))
                                .selected_text(if f.op == Op::Above { "above" } else { "below" })
                                .width(64.0)
                                .show_ui(ui, |ui| {
                                    crate::widgets::option_value(ui, &mut f.op, Op::Above, "above");
                                    crate::widgets::option_value(ui, &mut f.op, Op::Below, "below");
                                });
                            ui.add(egui::DragValue::new(&mut f.value).speed(0.1).max_decimals(4));
                            if ui.small_button(ph::X).on_hover_text("Remove filter").clicked() {
                                remove = Some(k);
                            }
                        });
                    });
            }
            if let Some(k) = remove {
                self.screener.setup.filters.remove(k);
            }
            if crate::widgets::button(
                ui,
                crate::widgets::Kind::Secondary,
                true,
                Some(ph::FUNNEL),
                "Add filter",
            )
            .clicked()
            {
                self.screener.setup.filters.push(Filter {
                    field: Field::Change,
                    op: Op::Above,
                    value: 0.0,
                });
            }
        });
        ui.add_space(6.0);
        // ---- table
        let cols: Vec<Field> = Field::ALL
            .into_iter()
            .filter(|f| *f != Field::Script || self.screener.setup.script != ScreenScript::None)
            .collect();
        let sym_w = 130.0;
        let col_w = ((ui.available_width() - sym_w) / cols.len() as f32).clamp(96.0, 130.0);
        let table_w = sym_w + col_w * cols.len() as f32;
        let row_h = 24.0;
        let mut open = None;
        egui::ScrollArea::horizontal().id_salt("screener-h").show(ui, |ui| {
            ui.set_min_width(table_w);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                let (r, _) = ui.allocate_exact_size(vec2(sym_w, row_h), Sense::hover());
                ui.painter().text(
                    r.left_center() + vec2(6.0, 0.0),
                    egui::Align2::LEFT_CENTER,
                    "Symbol",
                    theme::medium(12.0),
                    t.text3,
                );
                for f in &cols {
                    let (r, resp) = ui.allocate_exact_size(vec2(col_w, row_h), Sense::click());
                    let (sf, desc) = self.screener.sort;
                    let label = if sf == *f {
                        format!("{} {}", f.name(), if desc { "↓" } else { "↑" })
                    } else {
                        f.name().to_string()
                    };
                    ui.painter().text(
                        r.right_center() - vec2(6.0, 0.0),
                        egui::Align2::RIGHT_CENTER,
                        label,
                        theme::medium(12.0),
                        if sf == *f { t.accent } else { t.text3 },
                    );
                    if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                        self.screener.sort = (*f, if sf == *f { !desc } else { true });
                    }
                }
            });
            let filters = self.screener.setup.filters.clone();
            let (sf, desc) = self.screener.sort;
            let mut rows: Vec<&Row> = self
                .screener
                .rows
                .iter()
                .filter(|r| !r.done || r.error.is_some() || r.passes(&filters))
                .collect();
            rows.sort_by(|a, b| {
                let (x, y) = (a.get(sf), b.get(sf));
                let o = match (x.is_finite(), y.is_finite()) {
                    (true, true) => x.partial_cmp(&y).unwrap_or(std::cmp::Ordering::Equal),
                    (true, false) => std::cmp::Ordering::Greater,
                    (false, true) => std::cmp::Ordering::Less,
                    _ => std::cmp::Ordering::Equal,
                };
                if desc { o.reverse() } else { o }
            });
            egui::ScrollArea::vertical()
                .id_salt("screener-rows")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    for r in rows {
                        let (rect, resp) = ui.allocate_exact_size(vec2(table_w, row_h), Sense::click());
                        if resp.hovered() {
                            ui.painter().rect_filled(rect, 4, t.hover_wash());
                        }
                        let p = ui.painter();
                        p.text(
                            rect.left_center() + vec2(6.0, 0.0),
                            egui::Align2::LEFT_CENTER,
                            r.symbol.ticker(),
                            theme::medium(12.5),
                            t.text,
                        );
                        if let Some(e) = &r.error {
                            p.text(
                                rect.left_center() + vec2(sym_w, 0.0),
                                egui::Align2::LEFT_CENTER,
                                e.lines().next().unwrap_or(e),
                                theme::regular(12.0),
                                t.text3,
                            );
                        } else if !r.done {
                            p.text(
                                rect.left_center() + vec2(sym_w, 0.0),
                                egui::Align2::LEFT_CENTER,
                                "…",
                                theme::regular(12.0),
                                t.text3,
                            );
                        } else {
                            for (k, f) in cols.iter().enumerate() {
                                let v = r.get(*f);
                                let text = if !v.is_finite() {
                                    "—".to_string()
                                } else if f.pct() {
                                    crate::format::signed_pct(v)
                                } else if *f == Field::Volume {
                                    crate::format::compact(v)
                                } else if *f == Field::Last {
                                    crate::format::price(v, r.symbol.precision().max(2))
                                } else {
                                    format!("{v:.2}")
                                };
                                let col = if f.pct() && v.is_finite() && *f != Field::Atr {
                                    if v >= 0.0 { t.pos } else { t.neg }
                                } else {
                                    t.text
                                };
                                let x = rect.left() + sym_w + col_w * (k + 1) as f32 - 6.0;
                                p.text(
                                    egui::pos2(x, rect.center().y),
                                    egui::Align2::RIGHT_CENTER,
                                    text,
                                    theme::regular(12.5),
                                    col,
                                );
                            }
                        }
                        let resp = match &r.error {
                            Some(e) => resp.on_hover_text(e),
                            None => resp,
                        };
                        if resp.clicked() {
                            open = Some(r.symbol.clone());
                        }
                    }
                });
        });
        if let Some(s) = open {
            self.open_symbol_synced(s);
        }
    }

    /// The script column: none, a built-in, or a Pine file, and its plot.
    fn script_picker(&mut self, ui: &mut Ui, _t: &Theme) {
        let current = match &self.screener.setup.script {
            ScreenScript::None => "No script".to_string(),
            ScreenScript::Builtin(id, _) => opentern_series::builtins::by_id(id)
                .map(|m| m.short.to_string())
                .unwrap_or_else(|| id.clone()),
            ScreenScript::Pine(f, _) => f.clone(),
        };
        crate::widgets::combo("screen-script")
            .selected_text(format!("{}  {current}", ph::FUNCTION))
            .width(180.0)
            .height(360.0)
            .show_ui(ui, |ui| {
                if crate::widgets::option(ui, self.screener.setup.script == ScreenScript::None, "No script").clicked() {
                    self.screener.setup.script = ScreenScript::None;
                }
                let pine: Vec<String> = self
                    .script_files()
                    .into_iter()
                    .filter(|f| f.ends_with(".pine"))
                    .collect();
                if !pine.is_empty() {
                    ui.separator();
                }
                for f in pine {
                    let s = ScreenScript::Pine(f.clone(), 0);
                    if crate::widgets::option(
                        ui,
                        matches!(&self.screener.setup.script, ScreenScript::Pine(x, _) if *x == f),
                        &f,
                    )
                    .clicked()
                    {
                        self.screener.setup.script = s;
                    }
                }
                ui.separator();
                for m in opentern_series::builtins::all().iter().filter(|m| !m.plots.is_empty()) {
                    let on = matches!(&self.screener.setup.script, ScreenScript::Builtin(x, _) if x == m.id);
                    if crate::widgets::option(ui, on, m.name).clicked() {
                        self.screener.setup.script = ScreenScript::Builtin(m.id.into(), 0);
                    }
                }
            });
        // Which plot of a built-in to read.
        if let ScreenScript::Builtin(id, plot) = &mut self.screener.setup.script
            && let Some(m) = opentern_series::builtins::by_id(id)
            && m.plots.len() > 1
        {
            crate::widgets::combo("screen-plot")
                .selected_text(m.plots.get(*plot).map(|p| p.title).unwrap_or("Plot"))
                .width(110.0)
                .show_ui(ui, |ui| {
                    for (k, p) in m.plots.iter().enumerate() {
                        crate::widgets::option_value(ui, plot, k, p.title);
                    }
                });
        }
        if let ScreenScript::Pine(_, plot) = &mut self.screener.setup.script {
            ui.add(egui::DragValue::new(plot).range(0..=63).prefix("plot "))
                .on_hover_text("Which plot to show (0 is the first)");
        }
        let _ = Layout::left_to_right(Align::Center);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bars(n: usize) -> Vec<Bar> {
        (0..n)
            .map(|i| {
                let c = 100.0 + i as f64 * 0.5;
                Bar {
                    time: i as i64 * 86_400_000,
                    open: c - 0.2,
                    high: c + 1.0,
                    low: c - 1.0,
                    close: c,
                    volume: 1000.0 + i as f64,
                }
            })
            .collect()
    }

    #[test]
    fn measures_and_filters() {
        let tf = Timeframe::parse("1D").unwrap();
        let v = measure(&bars(300), tf, &jiff::tz::TimeZone::UTC, &Job::None).unwrap();
        assert!((v[&Field::Last] - 249.5).abs() < 1e-9);
        assert!(v[&Field::Change] > 0.0);
        assert!(v[&Field::Rsi] > 70.0, "a steady climb is overbought");
        assert!(v[&Field::Sma50] > 0.0 && v[&Field::Sma200] > v[&Field::Sma50]);
        let row = Row {
            symbol: Symbol::bare("X"),
            values: v,
            error: None,
            done: true,
        };
        let f = |field, op, value| Filter { field, op, value };
        assert!(row.passes(&[f(Field::Rsi, Op::Above, 50.0)]));
        assert!(!row.passes(&[f(Field::Rsi, Op::Above, 50.0), f(Field::Change, Op::Below, 0.0)]));
    }

    #[test]
    fn pine_script_column() {
        let tf = Timeframe::parse("1D").unwrap();
        let env = opentern_pine::Env {
            tickerid: "X".into(),
            description: String::new(),
            mintick: 0.01,
            currency: "USD".into(),
            kind: "stock".into(),
            min_qty: 0.0,
            timezone: "UTC".into(),
            libraries: Vec::new(),
            feeds: Default::default(),
        };
        let src = "//@version=5\nindicator(\"t\")\nplot(close - close[10])\n";
        let job = Job::Pine(src.into(), "t.pine".into(), env, 0);
        let v = measure(&bars(100), tf, &jiff::tz::TimeZone::UTC, &job).unwrap();
        assert!((v[&Field::Script] - 5.0).abs() < 1e-9);
    }
}

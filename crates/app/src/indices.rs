//! Custom indices: a named basket of symbols ("Nifty Auto"), weighted by
//! market cap (or equally), charted like any symbol and measured on one
//! board: the index's own numbers on top, then every member's, as a heat
//! table or a treemap.
//!
//! An index is a formula (`w1*NSE:MARUTI + w2*NSE:M&M + …`) registered with
//! the hub under `CUSTOM:<TICKER>` (`opentern_data::expr::set_customs`), so
//! the chart, quotes, indicators and alerts all work on it unchanged. The
//! weights are the members' share counts (or 1 ÷ price for equal weight),
//! scaled so the index is `base` (1000) on the first day every member has
//! a price. They're fixed when the members are, like a fixed-share index.
//!
//! Members' daily bars come by `Request::Fetch` (seqs from `SEQ_BASE`), their
//! fundamentals by `Request::Fundamentals` (Yahoo, `opentern_data::fundamentals`).

use crate::app::App;
use crate::dialogs::{Done, Modal, frame};
use crate::icons::ph;
use crate::motion::lerp_color;
use crate::theme::{self, Theme};
use crate::widgets as w;
use egui::{Align2, Color32, CornerRadius, Rect, Sense, Stroke, Ui, pos2, vec2};
use opentern_core::{AssetKind, Bar, Symbol, Timeframe};
use opentern_data::Request;
use opentern_data::fundamentals::Fundamentals;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::time::{Duration, Instant};

/// Fetch seqs for members' bars: `SEQ_BASE + n`.
pub const SEQ_BASE: u64 = 1 << 46;
/// Daily bars per member: a 200-day average with room to warm up.
const BARS: usize = 400;
/// Members' data is fetched again when it's older than this.
const STALE: Duration = Duration::from_secs(15 * 60);
const SETTINGS_KEY: &str = "custom_indices";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Weighting {
    #[default]
    MarketCap,
    Equal,
}

/// One index, as saved (settings key `custom_indices`).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct IndexDef {
    /// The ticker: the chart's symbol is `CUSTOM:<ticker>`.
    pub ticker: String,
    pub name: String,
    pub members: Vec<Symbol>,
    pub weighting: Weighting,
    pub base: f64,
    /// Units of each member (parallel to `members`); empty until worked
    /// out from the members' prices and share counts.
    pub units: Vec<f64>,
    /// When the index stood at `base`.
    pub base_time: i64,
    /// Market cap was asked for but some member had no share count.
    pub equal_fallback: bool,
}

impl Default for IndexDef {
    fn default() -> Self {
        IndexDef {
            ticker: String::new(),
            name: String::new(),
            members: Vec::new(),
            weighting: Weighting::MarketCap,
            base: 1000.0,
            units: Vec::new(),
            base_time: 0,
            equal_fallback: false,
        }
    }
}

impl IndexDef {
    pub fn id(&self) -> String {
        format!("{}{}", opentern_data::expr::CUSTOM, self.ticker)
    }

    pub fn ready(&self) -> bool {
        !self.units.is_empty() && self.units.len() == self.members.len()
    }

    /// The formula the hub charts.
    pub fn formula(&self) -> Option<String> {
        self.ready().then(|| {
            self.members
                .iter()
                .zip(&self.units)
                .map(|(m, u)| format!("{u}*{}", m.id))
                .collect::<Vec<_>>()
                .join(" + ")
        })
    }

    /// The index as a chart symbol (hours and currency of its first member).
    pub fn symbol(&self) -> Symbol {
        let first = self.members.first();
        Symbol {
            id: self.id(),
            description: self.name.clone(),
            kind: AssetKind::Index,
            currency: first.map(|s| s.currency.clone()).unwrap_or_default(),
            tick_size: 0.01,
            min_qty: 0.0,
            timezone: first.map(|s| s.timezone.clone()).unwrap_or_else(|| "UTC".into()),
        }
    }
}

/// A ticker from a name: "Nifty Auto" → "NIFTYAUTO".
fn ticker_for(name: &str, taken: &[String]) -> String {
    let base: String = name
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_uppercase())
        .take(20)
        .collect();
    let base = if base.is_empty() { "INDEX".to_string() } else { base };
    let mut t = base.clone();
    let mut n = 2;
    while taken.contains(&t) {
        t = format!("{base}{n}");
        n += 1;
    }
    t
}

/// What's been fetched for one member.
#[derive(Default)]
struct MemberData {
    bars: Option<Result<Vec<Bar>, String>>,
    fund: Option<Result<Fundamentals, String>>,
    asked: Option<Instant>,
}

/// The measures on the board, in the order shown: how it's moved, what
/// it costs, how the business is doing, where the price sits.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Metric {
    Ret1M,
    Ret3M,
    Pe,
    EvEbitda,
    RevGrowth,
    GrossMargin,
    EbitdaMargin,
    EbitMargin,
    PatMargin,
    Rsi,
    Sma50,
    Sma200,
}

impl Metric {
    pub const ALL: [Metric; 12] = [
        Metric::Ret1M,
        Metric::Ret3M,
        Metric::Pe,
        Metric::EvEbitda,
        Metric::RevGrowth,
        Metric::GrossMargin,
        Metric::EbitdaMargin,
        Metric::EbitMargin,
        Metric::PatMargin,
        Metric::Rsi,
        Metric::Sma50,
        Metric::Sma200,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Metric::Ret1M => "1M return",
            Metric::Ret3M => "3M return",
            Metric::Pe => "P/E",
            Metric::EvEbitda => "EV/EBITDA",
            Metric::RevGrowth => "Revenue growth",
            Metric::GrossMargin => "Gross margin",
            Metric::EbitdaMargin => "EBITDA margin",
            Metric::EbitMargin => "EBIT margin",
            Metric::PatMargin => "PAT margin",
            Metric::Rsi => "RSI (14)",
            Metric::Sma50 => "vs 50 SMA",
            Metric::Sma200 => "vs 200 SMA",
        }
    }

    /// The column title where room is short.
    fn short(self) -> &'static str {
        match self {
            Metric::Ret1M => "1M",
            Metric::Ret3M => "3M",
            Metric::RevGrowth => "Rev growth",
            Metric::GrossMargin => "Gross",
            Metric::EbitdaMargin => "EBITDA",
            Metric::EbitMargin => "EBIT",
            Metric::PatMargin => "PAT",
            Metric::Rsi => "RSI",
            Metric::Sma50 => "50 SMA",
            Metric::Sma200 => "200 SMA",
            m => m.name(),
        }
    }

    fn group(self) -> &'static str {
        match self {
            Metric::Ret1M | Metric::Ret3M => "Returns",
            Metric::Pe | Metric::EvEbitda => "Valuation",
            Metric::RevGrowth | Metric::GrossMargin | Metric::EbitdaMargin | Metric::EbitMargin | Metric::PatMargin => {
                "Growth and margins"
            }
            Metric::Rsi | Metric::Sma50 | Metric::Sma200 => "Technicals",
        }
    }

    fn tip(self) -> &'static str {
        match self {
            Metric::Ret1M => "Change over the last month",
            Metric::Ret3M => "Change over the last three months",
            Metric::Pe => "Price over trailing earnings. For the index: total market cap over total earnings",
            Metric::EvEbitda => "Enterprise value over trailing EBITDA. For the index: total EV over total EBITDA",
            Metric::RevGrowth => "Revenue, year on year. For the index: the members' revenue together",
            Metric::GrossMargin | Metric::EbitdaMargin | Metric::EbitMargin | Metric::PatMargin => {
                "Trailing twelve months. For the index: weighted by revenue"
            }
            Metric::Rsi => "Relative Strength Index, 14 days",
            Metric::Sma50 => "How far the price is above or below its 50-day average",
            Metric::Sma200 => "How far the price is above or below its 200-day average",
        }
    }

    fn format(self, v: f64) -> String {
        if !v.is_finite() {
            return "—".into();
        }
        match self {
            Metric::Pe | Metric::EvEbitda | Metric::Rsi => format!("{v:.1}"),
            Metric::GrossMargin | Metric::EbitdaMargin | Metric::EbitMargin | Metric::PatMargin => {
                format!("{v:.1}%")
            }
            _ => crate::format::signed_pct(v),
        }
    }

    /// Where `v` sits among `all`, from -1 (worst) to 1 (best). Moves and
    /// distances go by their sign and size; ratios and margins by rank.
    fn score(self, v: f64, all: &[f64]) -> Option<f32> {
        if !v.is_finite() {
            return None;
        }
        match self {
            Metric::Ret1M | Metric::Ret3M | Metric::Sma50 | Metric::Sma200 | Metric::RevGrowth => {
                let max = all.iter().filter(|x| x.is_finite()).fold(0.0f64, |m, x| m.max(x.abs()));
                Some(if max > 0.0 {
                    (v / max).clamp(-1.0, 1.0) as f32
                } else {
                    0.0
                })
            }
            Metric::Rsi => Some(((v - 50.0) / 25.0).clamp(-1.0, 1.0) as f32),
            _ => {
                let lower_better = matches!(self, Metric::Pe | Metric::EvEbitda);
                // A loss-maker's P/E is the worst, whatever its number.
                let key = |x: f64| {
                    if lower_better {
                        if x <= 0.0 { f64::NEG_INFINITY } else { -x }
                    } else {
                        x
                    }
                };
                let vals: Vec<f64> = all.iter().copied().filter(|x| x.is_finite()).map(key).collect();
                if vals.len() < 2 {
                    return Some(0.0);
                }
                let k = key(v);
                let below = vals.iter().filter(|x| **x < k).count() as f32;
                let equal = vals.iter().filter(|x| **x == k).count() as f32;
                let rank = (below + (equal - 1.0) / 2.0) / (vals.len() - 1) as f32;
                Some(rank * 2.0 - 1.0)
            }
        }
    }
}

/// A score's cell colour: soft green for good, soft red for bad.
fn heat(t: &Theme, s: Option<f32>) -> Color32 {
    match s {
        None => Color32::TRANSPARENT,
        Some(s) => {
            let c = if s >= 0.0 { t.pos } else { t.neg };
            lerp_color(t.bg, c, 0.08 + 0.42 * s.abs())
        }
    }
}

/// One row of the board.
#[derive(Clone, Debug)]
pub struct BoardRow {
    pub symbol: Symbol,
    pub weight: f64,
    pub price: f64,
    pub change: f64,
    pub market_cap: f64,
    pub values: HashMap<Metric, f64>,
    pub error: Option<String>,
}

impl BoardRow {
    fn get(&self, m: Metric) -> f64 {
        self.values.get(&m).copied().unwrap_or(f64::NAN)
    }
}

/// Everything the board shows for one index, worked out when data changes.
#[derive(Clone, Debug, Default)]
pub struct Board {
    pub rows: Vec<BoardRow>,
    pub index: HashMap<Metric, f64>,
    pub value: f64,
    pub change: f64,
    pub series: Vec<f64>,
    /// Members still loading.
    pub waiting: usize,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum View {
    #[default]
    Table,
    Map,
}

pub struct Indices {
    pub defs: Vec<IndexDef>,
    /// The index on the board.
    pub open: Option<String>,
    data: HashMap<String, MemberData>,
    seq: u64,
    pub view: View,
    /// What colours the treemap.
    pub metric: Metric,
    /// The table's order: a metric (or `None` = weight), descending?
    sort: (Option<Metric>, bool),
    version: u64,
    board: Option<(u64, String, Board)>,
    /// Put this index on the chart once its weights are worked out.
    pub(crate) chart_when_ready: Option<String>,
}

impl Indices {
    pub fn load(store: &opentern_core::Store) -> Indices {
        let defs: Vec<IndexDef> = store
            .value(SETTINGS_KEY)
            .and_then(|j| serde_json::from_str(&j).ok())
            .unwrap_or_default();
        let ix = Indices {
            open: defs.first().map(|d| d.ticker.clone()),
            defs,
            data: HashMap::new(),
            seq: 0,
            view: View::Table,
            metric: Metric::Ret1M,
            sort: (None, true),
            version: 0,
            board: None,
            chart_when_ready: None,
        };
        ix.register();
        ix
    }

    /// Tells the hub every index's formula.
    fn register(&self) {
        opentern_data::expr::set_customs(self.defs.iter().filter_map(|d| d.formula().map(|f| (d.id(), f))));
    }

    pub fn by_ticker(&self, t: &str) -> Option<&IndexDef> {
        self.defs.iter().find(|d| d.ticker == t)
    }

    /// The indices ready to chart (for live quotes).
    pub fn symbols(&self) -> Vec<Symbol> {
        self.defs.iter().filter(|d| d.ready()).map(|d| d.symbol()).collect()
    }
}

/// The close at or before `t`.
fn close_at(bars: &[Bar], t: i64) -> f64 {
    crate::screener::close_at(bars, t).unwrap_or(f64::NAN)
}

/// Units per member so the index is `base` on the first day all have a
/// price: share counts (market cap) or 1 ÷ price (equal). Returns
/// (units, base time, fell back to equal).
pub fn work_out_units(members: &[(&[Bar], f64)], weighting: Weighting, base: f64) -> Option<(Vec<f64>, i64, bool)> {
    let t0 = members
        .iter()
        .map(|(b, _)| b.first().map(|x| x.time))
        .collect::<Option<Vec<_>>>()?;
    let t0 = *t0.iter().max()?;
    let p0: Vec<f64> = members.iter().map(|(b, _)| close_at(b, t0)).collect();
    if p0.iter().any(|p| !p.is_finite() || *p <= 0.0) {
        return None;
    }
    let caps = weighting == Weighting::MarketCap;
    let have_shares = members.iter().all(|(_, s)| s.is_finite() && *s > 0.0);
    let raw: Vec<f64> = if caps && have_shares {
        members.iter().map(|(_, s)| *s).collect()
    } else {
        p0.iter().map(|p| 1.0 / p).collect()
    };
    let total: f64 = raw.iter().zip(&p0).map(|(u, p)| u * p).sum();
    if total <= 0.0 {
        return None;
    }
    let units = raw.iter().map(|u| u * base / total).collect();
    Some((units, t0, caps && !have_shares))
}

/// The index's daily closes from its members' bars (on days all trade).
fn index_series(members: &[&[Bar]], units: &[f64]) -> Vec<Bar> {
    let mut by_time: BTreeMap<i64, (usize, f64, f64, f64, f64)> = BTreeMap::new();
    for (bars, u) in members.iter().zip(units) {
        for b in bars.iter() {
            let e = by_time.entry(b.time).or_insert((0, 0.0, 0.0, 0.0, 0.0));
            e.0 += 1;
            e.1 += u * b.open;
            e.2 += u * b.high;
            e.3 += u * b.low;
            e.4 += u * b.close;
        }
    }
    by_time
        .into_iter()
        .filter(|(_, e)| e.0 == members.len())
        .map(|(time, (_, o, h, l, c))| Bar {
            time,
            open: o,
            high: h.max(o).max(c),
            low: l.min(o).min(c),
            close: c,
            volume: 0.0,
        })
        .collect()
}

/// Price-based measures of one series.
fn price_measures(bars: &[Bar], tz: &jiff::tz::TimeZone, out: &mut HashMap<Metric, f64>) {
    let Some(last) = bars.last() else { return };
    let day = 86_400_000;
    for (m, days) in [(Metric::Ret1M, 30), (Metric::Ret3M, 91)] {
        let c = close_at(bars, last.time - days * day);
        if c.is_finite() && c != 0.0 {
            out.insert(m, (last.close / c - 1.0) * 100.0);
        }
    }
    let tf = Timeframe::parse("1D").expect("valid");
    let sess = opentern_series::study::sessions(bars, tf, tz);
    out.insert(Metric::Rsi, crate::screener::builtin_last("rsi", None, bars, &sess, tf));
    for (m, len) in [(Metric::Sma50, 50), (Metric::Sma200, 200)] {
        let sma = crate::screener::builtin_last("sma", Some(len), bars, &sess, tf);
        if sma.is_finite() && sma != 0.0 {
            out.insert(m, (last.close / sma - 1.0) * 100.0);
        }
    }
}

/// The index's fundamentals from its members': totals over totals, and
/// margins weighted by revenue.
fn index_fundamentals(funds: &[&Fundamentals], out: &mut HashMap<Metric, f64>) {
    let sum = |f: &dyn Fn(&Fundamentals) -> Option<(f64, f64)>| {
        let (mut a, mut b, mut n) = (0.0, 0.0, 0);
        for x in funds {
            if let Some((p, q)) = f(x)
                && p.is_finite()
                && q.is_finite()
            {
                a += p;
                b += q;
                n += 1;
            }
        }
        if n > 0 && b != 0.0 { a / b } else { f64::NAN }
    };
    out.insert(Metric::Pe, sum(&|f| Some((f.market_cap, f.net_income))));
    out.insert(Metric::EvEbitda, sum(&|f| Some((f.enterprise_value, f.ebitda))));
    let g = sum(&|f| Some((f.revenue, f.revenue / (1.0 + f.revenue_growth))));
    out.insert(Metric::RevGrowth, (g - 1.0) * 100.0);
    for (m, get) in [
        (
            Metric::GrossMargin,
            (|f: &Fundamentals| f.gross_margin) as fn(&Fundamentals) -> f64,
        ),
        (Metric::EbitdaMargin, |f| f.ebitda_margin),
        (Metric::EbitMargin, |f| f.ebit_margin),
        (Metric::PatMargin, |f| f.pat_margin),
    ] {
        out.insert(m, sum(&|f| Some((get(f) * f.revenue, f.revenue))) * 100.0);
    }
}

fn member_fundamentals(f: &Fundamentals, out: &mut HashMap<Metric, f64>) {
    out.insert(Metric::Pe, f.pe);
    out.insert(Metric::EvEbitda, f.ev_ebitda);
    out.insert(Metric::RevGrowth, f.revenue_growth * 100.0);
    out.insert(Metric::GrossMargin, f.gross_margin * 100.0);
    out.insert(Metric::EbitdaMargin, f.ebitda_margin * 100.0);
    out.insert(Metric::EbitMargin, f.ebit_margin * 100.0);
    out.insert(Metric::PatMargin, f.pat_margin * 100.0);
}

impl App {
    fn index_def(&self) -> Option<&IndexDef> {
        let t = self.indices.open.as_ref()?;
        self.indices.by_ticker(t)
    }

    pub(crate) fn save_indices(&mut self) {
        if let Ok(j) = serde_json::to_string(&self.indices.defs) {
            let _ = self.store.set_value(SETTINGS_KEY, &j);
        }
        self.indices.register();
        self.indices.version += 1;
    }

    /// Asks for whatever the open index's members are missing.
    fn indices_want(&mut self) {
        let Some(def) = self.index_def().cloned() else { return };
        for m in def.members {
            let d = self.indices.data.entry(m.id.clone()).or_default();
            if d.asked.is_some_and(|a| a.elapsed() < STALE) {
                continue;
            }
            d.asked = Some(Instant::now());
            self.indices.seq += 1;
            self.hub.send(Request::Fetch {
                seq: SEQ_BASE + self.indices.seq,
                symbol: m.clone(),
                tf: Timeframe::parse("1D").expect("valid"),
                count: BARS,
            });
            self.hub.send(Request::Fundamentals {
                seq: self.indices.seq,
                symbol: m,
            });
        }
    }

    pub(crate) fn indices_fetched(&mut self, id: &str, result: Result<Vec<Bar>, String>) {
        let d = self.indices.data.entry(id.to_string()).or_default();
        // Keep what we had if a refresh fails.
        if result.is_ok() || !matches!(d.bars, Some(Ok(_))) {
            d.bars = Some(result);
        }
        self.indices.version += 1;
        self.indices_settle();
    }

    pub(crate) fn indices_fundamentals(&mut self, id: &str, result: Result<Fundamentals, String>) {
        let d = self.indices.data.entry(id.to_string()).or_default();
        if result.is_ok() || !matches!(d.fund, Some(Ok(_))) {
            d.fund = Some(result);
        }
        self.indices.version += 1;
        self.indices_settle();
    }

    /// Works out the weights of indices whose members have all arrived.
    fn indices_settle(&mut self) {
        let mut changed = Vec::new();
        for (k, def) in self.indices.defs.iter().enumerate() {
            if def.ready() || def.members.is_empty() {
                continue;
            }
            let mut ins = Vec::new();
            for m in &def.members {
                let Some(d) = self.indices.data.get(&m.id) else { break };
                let Some(Ok(bars)) = &d.bars else { break };
                // Shares only matter for market cap; a failed lookup counts
                // as "none" (equal weights then).
                let shares = match (&d.fund, def.weighting) {
                    (_, Weighting::Equal) => f64::NAN,
                    (Some(Ok(f)), _) => f.shares,
                    (Some(Err(_)), _) => f64::NAN,
                    (None, _) => break,
                };
                ins.push((bars.as_slice(), shares));
            }
            if ins.len() != def.members.len() {
                continue;
            }
            if let Some(r) = work_out_units(&ins, def.weighting, def.base) {
                changed.push((k, r));
            }
        }
        if changed.is_empty() {
            return;
        }
        let mut settled = Vec::new();
        for (k, (units, t0, fallback)) in changed {
            let d = &mut self.indices.defs[k];
            d.units = units;
            d.base_time = t0;
            d.equal_fallback = fallback;
            settled.push(d.clone());
        }
        self.save_indices();
        self.subscribe_quotes();
        // Charts already on an index see its new weights.
        let id = self.chart.symbol.id.clone();
        if let Some(def) = settled.iter().find(|d| d.id() == id) {
            self.open_symbol(def.symbol());
        }
        if let Some(t) = self.indices.chart_when_ready.take()
            && let Some(def) = self.indices.by_ticker(&t).cloned()
        {
            if !def.ready() {
                self.indices.chart_when_ready = Some(t);
            } else if def.id() != self.chart.symbol.id {
                self.open_symbol_synced(def.symbol());
            }
        }
    }

    /// The open index's board, worked out again only when data changed.
    fn index_board(&mut self) -> Option<Board> {
        let def = self.index_def()?.clone();
        let v = self.indices.version;
        if let Some((ver, t, b)) = &self.indices.board
            && *ver == v
            && *t == def.ticker
        {
            return Some(b.clone());
        }
        let tz = jiff::tz::TimeZone::get(&def.symbol().timezone).unwrap_or(jiff::tz::TimeZone::UTC);
        let mut board = Board::default();
        let mut all_bars: Vec<&[Bar]> = Vec::new();
        let mut funds: Vec<&Fundamentals> = Vec::new();
        let mut caps = Vec::new();
        for (k, m) in def.members.iter().enumerate() {
            let d = self.indices.data.get(&m.id);
            let mut row = BoardRow {
                symbol: m.clone(),
                weight: f64::NAN,
                price: f64::NAN,
                change: f64::NAN,
                market_cap: f64::NAN,
                values: HashMap::new(),
                error: None,
            };
            match d.and_then(|d| d.bars.as_ref()) {
                Some(Ok(bars)) if !bars.is_empty() => {
                    let n = bars.len();
                    row.price = bars[n - 1].close;
                    if n >= 2 && bars[n - 2].close != 0.0 {
                        row.change = (bars[n - 1].close / bars[n - 2].close - 1.0) * 100.0;
                    }
                    price_measures(bars, &tz, &mut row.values);
                    all_bars.push(bars);
                    if let Some(u) = def.units.get(k) {
                        caps.push((k, u * row.price));
                    }
                }
                Some(Ok(_)) => row.error = Some("No prices came back.".into()),
                Some(Err(e)) => row.error = Some(e.clone()),
                None => board.waiting += 1,
            }
            match d.and_then(|d| d.fund.as_ref()) {
                Some(Ok(f)) => {
                    member_fundamentals(f, &mut row.values);
                    row.market_cap = f.market_cap;
                    funds.push(f);
                }
                Some(Err(e)) if row.error.is_none() => {
                    row.values.insert(Metric::Pe, f64::NAN);
                    row.error = Some(format!("Fundamentals: {e}"));
                }
                None if row.error.is_none() && d.and_then(|d| d.bars.as_ref()).is_some() => board.waiting += 1,
                _ => {}
            }
            board.rows.push(row);
        }
        // Today's weights: each member's units at today's price.
        let total: f64 = caps.iter().map(|(_, c)| c).sum();
        if total > 0.0 {
            for (k, c) in caps {
                board.rows[k].weight = c / total * 100.0;
            }
        }
        if def.ready() && all_bars.len() == def.members.len() {
            let series = index_series(&all_bars, &def.units);
            if let Some(last) = series.last() {
                board.value = last.close;
                let n = series.len();
                if n >= 2 && series[n - 2].close != 0.0 {
                    board.change = (last.close / series[n - 2].close - 1.0) * 100.0;
                }
            }
            price_measures(&series, &tz, &mut board.index);
            board.series = series.iter().rev().take(90).rev().map(|b| b.close).collect();
        }
        index_fundamentals(&funds, &mut board.index);
        self.indices.board = Some((v, def.ticker.clone(), board.clone()));
        Some(board)
    }

    // ------------------------------------------------------------- tab

    pub(crate) fn indices_tab(&mut self, ui: &mut Ui, t: &Theme) {
        self.indices_want();
        // Waiting to chart an index whose weights are already known.
        if let Some(def) = self
            .indices
            .chart_when_ready
            .as_ref()
            .and_then(|t| self.indices.by_ticker(t))
            .filter(|d| d.ready())
            .cloned()
        {
            self.indices.chart_when_ready = None;
            self.open_symbol_synced(def.symbol());
        }
        let rect = ui.available_rect_before_wrap();
        let list_w = 190.0_f32.min(rect.width() * 0.3);
        let list = Rect::from_min_size(rect.min, vec2(list_w, rect.height()));
        let main = Rect::from_min_max(pos2(list.right() + 14.0, rect.top()), rect.max);
        ui.scope_builder(egui::UiBuilder::new().max_rect(list), |ui| self.indices_list(ui, t));
        ui.painter().vline(
            list.right() + 6.0,
            list.y_range(),
            Stroke::new(1.0, t.border.gamma_multiply(0.6)),
        );
        ui.scope_builder(egui::UiBuilder::new().max_rect(main), |ui| {
            ui.set_clip_rect(main.intersect(ui.clip_rect()));
            if self.index_def().is_some() {
                self.index_dashboard(ui, t);
            } else {
                ui.add_space(20.0);
                w::empty_state(ui, t, ph::CHART_PIE_SLICE, "No indices yet");
                ui.vertical_centered(|ui| {
                    if w::button(ui, w::Kind::Primary, true, Some(ph::PLUS), "New index").clicked() {
                        self.modal = Some(Modal::Index(Box::new(IndexEditor::new(None))));
                    }
                });
            }
        });
        ui.allocate_rect(rect, Sense::hover());
    }

    fn indices_list(&mut self, ui: &mut Ui, t: &Theme) {
        ui.horizontal(|ui| {
            ui.set_height(28.0);
            ui.label(egui::RichText::new("Indices").font(theme::semibold(13.5)));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if w::tool_button(ui, t, ph::PLUS, "New index", false, 26.0).clicked() {
                    self.modal = Some(Modal::Index(Box::new(IndexEditor::new(None))));
                }
            });
        });
        ui.add_space(4.0);
        let mut pick = None;
        egui::ScrollArea::vertical()
            .id_salt("indices-list")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for d in &self.indices.defs {
                    let on = self.indices.open.as_deref() == Some(&d.ticker);
                    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 42.0), Sense::click());
                    let p = ui.painter();
                    if on {
                        p.rect_filled(r, 8, t.accent_soft());
                    } else if resp.hovered() {
                        p.rect_filled(r, 8, t.hover_wash());
                    }
                    let name = crate::format::elide_to(p, &d.name, &theme::medium(13.0), r.width() - 20.0);
                    p.text(
                        r.left_top() + vec2(10.0, 7.0),
                        Align2::LEFT_TOP,
                        name,
                        theme::medium(13.0),
                        t.text,
                    );
                    // Demo quotes are made up per symbol and don't add up to the index.
                    let quote = if self.demo { None } else { self.quotes.get(&d.id()) };
                    let sub = match quote {
                        _ if !d.ready() => format!("{} symbols", d.members.len()),
                        Some((last, prev)) if *prev != 0.0 => {
                            format!(
                                "{}  {}",
                                crate::format::price(*last, 2),
                                crate::format::signed_pct((last / prev - 1.0) * 100.0)
                            )
                        }
                        _ => format!("{} symbols", d.members.len()),
                    };
                    p.text(
                        r.left_bottom() + vec2(10.0, -7.0),
                        Align2::LEFT_BOTTOM,
                        sub,
                        theme::regular(11.5),
                        t.text3,
                    );
                    if resp.clicked() {
                        pick = Some(d.clone());
                    }
                }
            });
        if let Some(d) = pick {
            self.indices.open = Some(d.ticker.clone());
            if d.ready() {
                self.open_symbol_synced(d.symbol());
            } else {
                self.indices.chart_when_ready = Some(d.ticker.clone());
            }
        }
    }

    fn index_dashboard(&mut self, ui: &mut Ui, t: &Theme) {
        let Some(def) = self.index_def().cloned() else { return };
        let Some(board) = self.index_board() else { return };
        // ---- header: name, value, change; view and edit on the right
        ui.horizontal(|ui| {
            ui.set_height(32.0);
            ui.spacing_mut().item_spacing.x = 10.0;
            let name = ui.add(
                egui::Label::new(egui::RichText::new(&def.name).font(theme::semibold(16.0))).sense(Sense::click()),
            );
            if name
                .on_hover_text("Open on the chart")
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .clicked()
                && def.ready()
            {
                self.open_symbol_synced(def.symbol());
            }
            let live = if self.demo {
                None
            } else {
                self.quotes.get(&def.id()).copied()
            };
            let (value, change) = match live {
                Some((l, p)) if p != 0.0 => (l, (l / p - 1.0) * 100.0),
                _ => (board.value, board.change),
            };
            if value.is_finite() && value > 0.0 {
                ui.label(egui::RichText::new(crate::format::price(value, 2)).font(theme::medium(15.0)));
                ui.label(
                    egui::RichText::new(crate::format::signed_pct(change))
                        .font(theme::medium(13.0))
                        .color(if change >= 0.0 { t.pos } else { t.neg }),
                );
            } else if board.waiting > 0 {
                ui.spinner();
            }
            if def.equal_fallback {
                ui.label(egui::RichText::new(ph::INFO).color(t.warn))
                    .on_hover_text("Weighted equally: some members have no share count for market-cap weights.");
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                if w::tool_button(ui, t, ph::PENCIL_SIMPLE, "Edit index", false, 28.0).clicked() {
                    self.modal = Some(Modal::Index(Box::new(IndexEditor::new(Some(&def)))));
                }
                let mut view = self.indices.view;
                w::segmented(ui, t, &mut view, &[(View::Table, "Table"), (View::Map, "Map")]);
                self.indices.view = view;
                if view == View::Map {
                    w::combo("index-map-metric")
                        .selected_text(self.indices.metric.name())
                        .width(140.0)
                        .show_ui(ui, |ui| {
                            for m in Metric::ALL {
                                w::option_value(ui, &mut self.indices.metric, m, m.name());
                            }
                        });
                }
            });
        });
        ui.add_space(6.0);
        // ---- the index's own numbers, one tile each
        kpi_strip(ui, t, &board);
        ui.add_space(8.0);
        let open = match self.indices.view {
            View::Table => self.index_table(ui, t, &board),
            View::Map => treemap(ui, t, &board, self.indices.metric),
        };
        if let Some(s) = open {
            self.open_symbol_synced(s);
        }
    }

    /// Members as rows, metrics as columns grouped under a band; each cell
    /// tinted by how it stands among the members.
    fn index_table(&mut self, ui: &mut Ui, t: &Theme, board: &Board) -> Option<Symbol> {
        let mut open = None;
        let sym_w = 150.0;
        let fixed = [("Weight", 64.0), ("Price", 82.0)];
        let fixed_w: f32 = fixed.iter().map(|(_, w)| w).sum();
        let col = ((ui.available_width() - sym_w - fixed_w) / Metric::ALL.len() as f32).clamp(66.0, 110.0);
        let total = sym_w + fixed_w + col * Metric::ALL.len() as f32;
        let row_h = 26.0;
        let x0 = sym_w + fixed_w;
        let border = Stroke::new(1.0, t.border.gamma_multiply(0.6));
        let columns: HashMap<Metric, Vec<f64>> = Metric::ALL
            .iter()
            .map(|m| (*m, board.rows.iter().map(|r| r.get(*m)).collect()))
            .collect();
        let mut rows: Vec<&BoardRow> = board.rows.iter().collect();
        let (sf, desc) = self.indices.sort;
        rows.sort_by(|a, b| {
            let (x, y) = match sf {
                Some(m) => (a.get(m), b.get(m)),
                None => (a.weight, b.weight),
            };
            let o = match (x.is_finite(), y.is_finite()) {
                (true, true) => x.partial_cmp(&y).unwrap_or(std::cmp::Ordering::Equal),
                (true, false) => std::cmp::Ordering::Greater,
                (false, true) => std::cmp::Ordering::Less,
                _ => std::cmp::Ordering::Equal,
            };
            if desc { o.reverse() } else { o }
        });
        let max_weight = board
            .rows
            .iter()
            .map(|r| r.weight)
            .filter(|w| w.is_finite())
            .fold(0.0, f64::max);
        egui::ScrollArea::horizontal().id_salt("index-table-h").show(ui, |ui| {
            ui.set_min_width(total);
            // Group band.
            let (band, _) = ui.allocate_exact_size(vec2(total, 18.0), Sense::hover());
            let p = ui.painter();
            let mut k = 0;
            while k < Metric::ALL.len() {
                let g = Metric::ALL[k].group();
                let n = Metric::ALL[k..].iter().take_while(|m| m.group() == g).count();
                let r = Rect::from_min_size(
                    band.min + vec2(x0 + col * k as f32 + 4.0, 0.0),
                    vec2(col * n as f32 - 8.0, 16.0),
                );
                p.hline(r.x_range(), r.bottom(), Stroke::new(1.0, t.border));
                let label = crate::format::elide_to(p, g, &theme::medium(11.0), r.width());
                p.text(r.center_top(), Align2::CENTER_TOP, label, theme::medium(11.0), t.text3);
                k += n;
            }
            // Column titles (click to sort).
            let (head, _) = ui.allocate_exact_size(vec2(total, row_h), Sense::hover());
            let font = theme::medium(12.0);
            let mut sort_click = None;
            let mut title = |ui: &mut Ui, r: Rect, text: &str, key: Option<Metric>, tip: &str, left: bool| {
                let on = self.indices.sort.0 == key;
                let room = r.width() - 12.0;
                let shown = crate::format::elide_to(ui.painter(), text, &font, room);
                let c = if on { t.accent } else { t.text3 };
                if left {
                    ui.painter().text(
                        r.left_center() + vec2(8.0, 0.0),
                        Align2::LEFT_CENTER,
                        &shown,
                        font.clone(),
                        c,
                    );
                } else {
                    ui.painter().text(
                        r.right_center() - vec2(8.0, 0.0),
                        Align2::RIGHT_CENTER,
                        &shown,
                        font.clone(),
                        c,
                    );
                }
                let resp = ui
                    .interact(r, ui.id().with(("index-head", text)), Sense::click())
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .on_hover_text(tip);
                if resp.clicked() {
                    sort_click = Some(key);
                }
            };
            title(
                ui,
                Rect::from_min_size(head.min, vec2(sym_w, row_h)),
                "Symbol",
                None,
                "Sort by weight",
                true,
            );
            let mut x = sym_w;
            for (name, wd) in fixed {
                let r = Rect::from_min_size(head.min + vec2(x, 0.0), vec2(wd, row_h));
                if name == "Weight" {
                    title(ui, r, name, None, "Today's weight in the index", false);
                } else {
                    ui.painter().text(
                        r.right_center() - vec2(8.0, 0.0),
                        Align2::RIGHT_CENTER,
                        name,
                        font.clone(),
                        t.text3,
                    );
                }
                x += wd;
            }
            for (k, m) in Metric::ALL.iter().enumerate() {
                let r = Rect::from_min_size(head.min + vec2(x0 + col * k as f32, 0.0), vec2(col, row_h));
                let text = if col >= 96.0 { m.name() } else { m.short() };
                title(ui, r, text, Some(*m), m.tip(), false);
            }
            if let Some(key) = sort_click {
                let (cur, d) = self.indices.sort;
                self.indices.sort = (key, if cur == key { !d } else { true });
            }
            ui.painter().hline(head.x_range(), head.bottom(), border);
            // The index's own row, then the members.
            egui::ScrollArea::vertical()
                .id_salt("index-table-v")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    let (r, resp) = ui.allocate_exact_size(vec2(total, row_h), Sense::click());
                    let p = ui.painter();
                    p.rect_filled(r, 4, t.hover_wash());
                    p.text(
                        r.left_center() + vec2(8.0, 0.0),
                        Align2::LEFT_CENTER,
                        "Index",
                        theme::semibold(12.5),
                        t.text,
                    );
                    p.text(
                        r.left_center() + vec2(sym_w + fixed[0].1 - 8.0, 0.0),
                        Align2::RIGHT_CENTER,
                        "100%",
                        theme::regular(12.5),
                        t.text2,
                    );
                    if board.value.is_finite() && board.value > 0.0 {
                        p.text(
                            r.left_center() + vec2(sym_w + fixed_w - 8.0, 0.0),
                            Align2::RIGHT_CENTER,
                            crate::format::price(board.value, 2),
                            theme::regular(12.5),
                            t.text,
                        );
                    }
                    for (k, m) in Metric::ALL.iter().enumerate() {
                        let v = board.index.get(m).copied().unwrap_or(f64::NAN);
                        let c = Rect::from_min_size(r.min + vec2(x0 + col * k as f32, 0.0), vec2(col, row_h));
                        p.text(
                            c.right_center() - vec2(8.0, 0.0),
                            Align2::RIGHT_CENTER,
                            m.format(v),
                            theme::semibold(12.5),
                            t.text,
                        );
                    }
                    if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                        open = self.index_def().filter(|d| d.ready()).map(|d| d.symbol());
                    }
                    for row in &rows {
                        let (r, resp) = ui.allocate_exact_size(vec2(total, row_h), Sense::click());
                        let p = ui.painter();
                        if resp.hovered() {
                            p.rect_filled(r, 4, t.hover_wash());
                        }
                        // Weight as a bar behind the ticker.
                        if row.weight.is_finite() && max_weight > 0.0 {
                            let wbar = (sym_w - 12.0) * (row.weight / max_weight) as f32;
                            p.rect_filled(
                                Rect::from_min_size(r.left_bottom() + vec2(6.0, -4.0), vec2(wbar, 2.0)),
                                1,
                                t.accent.gamma_multiply(0.55),
                            );
                        }
                        let ticker =
                            crate::format::elide_to(p, row.symbol.ticker(), &theme::medium(12.5), sym_w - 16.0);
                        p.text(
                            r.left_center() + vec2(8.0, -1.0),
                            Align2::LEFT_CENTER,
                            ticker,
                            theme::medium(12.5),
                            t.text,
                        );
                        let mut x = sym_w;
                        let cells = [
                            (
                                if row.weight.is_finite() {
                                    format!("{:.1}%", row.weight)
                                } else {
                                    "—".into()
                                },
                                t.text2,
                            ),
                            (
                                if row.price.is_finite() {
                                    crate::format::price(row.price, row.symbol.precision().max(2))
                                } else {
                                    "—".into()
                                },
                                t.text,
                            ),
                        ];
                        for ((text, c), (_, wd)) in cells.into_iter().zip(fixed) {
                            p.text(
                                r.left_center() + vec2(x + wd - 8.0, 0.0),
                                Align2::RIGHT_CENTER,
                                text,
                                theme::regular(12.5),
                                c,
                            );
                            x += wd;
                        }
                        if row.error.is_some() && row.values.is_empty() {
                            p.text(
                                r.left_center() + vec2(x0 + 8.0, 0.0),
                                Align2::LEFT_CENTER,
                                row.error.as_deref().unwrap_or_default(),
                                theme::regular(12.0),
                                t.text3,
                            );
                        } else {
                            for (k, m) in Metric::ALL.iter().enumerate() {
                                let v = row.get(*m);
                                let c = Rect::from_min_size(r.min + vec2(x0 + col * k as f32, 0.0), vec2(col, row_h));
                                p.rect_filled(c.shrink2(vec2(1.0, 1.5)), 4, heat(t, m.score(v, &columns[m])));
                                p.text(
                                    c.right_center() - vec2(8.0, 0.0),
                                    Align2::RIGHT_CENTER,
                                    m.format(v),
                                    theme::regular(12.5),
                                    if v.is_finite() { t.text } else { t.text3 },
                                );
                            }
                        }
                        let tip = match (&row.error, row.market_cap.is_finite()) {
                            (Some(e), _) => e.clone(),
                            (None, true) => format!(
                                "{}\nMarket cap {}",
                                row.symbol.id,
                                crate::format::compact(row.market_cap)
                            ),
                            (None, false) => row.symbol.id.clone(),
                        };
                        if resp.on_hover_text(tip).clicked() {
                            open = Some(row.symbol.clone());
                        }
                    }
                });
        });
        open
    }
}

/// The index's metrics as a row of small tiles (wrapping when narrow).
fn kpi_strip(ui: &mut Ui, t: &Theme, board: &Board) {
    let n = Metric::ALL.len() as f32;
    let gap = 6.0;
    let per_row = if ui.available_width() / n >= 84.0 {
        n
    } else {
        (n / 2.0).ceil()
    };
    let tile_w = ((ui.available_width() - gap * (per_row - 1.0)) / per_row).floor();
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(gap, gap);
        for m in Metric::ALL {
            let v = board.index.get(&m).copied().unwrap_or(f64::NAN);
            let (r, resp) = ui.allocate_exact_size(vec2(tile_w, 44.0), Sense::hover());
            let p = ui.painter();
            p.rect(
                r,
                8,
                t.card,
                Stroke::new(1.0, t.border.gamma_multiply(0.6)),
                egui::StrokeKind::Inside,
            );
            let label = crate::format::elide_to(p, m.name(), &theme::regular(11.0), r.width() - 16.0);
            p.text(
                r.left_top() + vec2(9.0, 7.0),
                Align2::LEFT_TOP,
                label,
                theme::regular(11.0),
                t.text3,
            );
            let signed = matches!(
                m,
                Metric::Ret1M | Metric::Ret3M | Metric::RevGrowth | Metric::Sma50 | Metric::Sma200
            );
            let c = if !v.is_finite() {
                t.text3
            } else if signed {
                if v >= 0.0 { t.pos } else { t.neg }
            } else {
                t.text
            };
            p.text(
                r.left_bottom() + vec2(9.0, -6.0),
                Align2::LEFT_BOTTOM,
                m.format(v),
                theme::semibold(14.0),
                c,
            );
            resp.on_hover_text(m.tip());
        }
    });
}

/// Squarified treemap: rectangles for `areas` (largest first) in `r`.
pub fn squarify(areas: &[f64], r: Rect) -> Vec<Rect> {
    let total: f64 = areas.iter().sum();
    if total <= 0.0 || r.area() <= 0.0 {
        return vec![Rect::NOTHING; areas.len()];
    }
    let scale = r.area() as f64 / total;
    let a: Vec<f64> = areas.iter().map(|x| x * scale).collect();
    let worst = |row: &[f64], side: f64| {
        let s: f64 = row.iter().sum();
        let (lo, hi) = row
            .iter()
            .fold((f64::INFINITY, 0.0f64), |(l, h), x| (l.min(*x), h.max(*x)));
        (side * side * hi / (s * s)).max(s * s / (side * side * lo))
    };
    let mut out = Vec::with_capacity(a.len());
    let mut free = r;
    let mut i = 0;
    while i < a.len() {
        let side = free.width().min(free.height()) as f64;
        let mut j = i + 1;
        while j < a.len() && worst(&a[i..=j], side) <= worst(&a[i..j], side) {
            j += 1;
        }
        let s: f64 = a[i..j].iter().sum();
        if free.width() >= free.height() {
            let wd = (s / free.height() as f64) as f32;
            let mut y = free.top();
            for x in &a[i..j] {
                let h = (*x / wd as f64) as f32;
                out.push(Rect::from_min_size(pos2(free.left(), y), vec2(wd, h)));
                y += h;
            }
            free.min.x += wd;
        } else {
            let ht = (s / free.width() as f64) as f32;
            let mut x = free.left();
            for v in &a[i..j] {
                let wd = (*v / ht as f64) as f32;
                out.push(Rect::from_min_size(pos2(x, free.top()), vec2(wd, ht)));
                x += wd;
            }
            free.min.y += ht;
        }
        i = j;
    }
    out
}

/// Members as tiles sized by weight, coloured by `metric`.
fn treemap(ui: &mut Ui, t: &Theme, board: &Board, metric: Metric) -> Option<Symbol> {
    let area = ui.available_rect_before_wrap();
    let resp = ui.allocate_rect(area, Sense::click());
    let mut rows: Vec<&BoardRow> = board
        .rows
        .iter()
        .filter(|r| r.weight.is_finite() && r.weight > 0.0)
        .collect();
    rows.sort_by(|a, b| b.weight.partial_cmp(&a.weight).unwrap_or(std::cmp::Ordering::Equal));
    if rows.is_empty() {
        if board.waiting > 0 {
            ui.put(area, egui::Spinner::new());
        }
        return None;
    }
    let all: Vec<f64> = board.rows.iter().map(|r| r.get(metric)).collect();
    let rects = squarify(&rows.iter().map(|r| r.weight).collect::<Vec<_>>(), area);
    let p = ui.painter_at(area);
    let hover = resp.hover_pos();
    let mut open = None;
    for (row, r) in rows.iter().zip(rects) {
        let r = r.shrink(1.5);
        let v = row.get(metric);
        let fill = match metric.score(v, &all) {
            Some(s) => heat(t, Some(s)),
            None => t.hover_wash(),
        };
        p.rect_filled(r, 6, fill);
        let hot = hover.is_some_and(|h| r.contains(h));
        if hot {
            p.rect_stroke(r, 6, Stroke::new(1.5, t.text2), egui::StrokeKind::Inside);
        }
        if r.width() > 46.0 && r.height() > 30.0 {
            let big = (r.width().min(r.height() * 2.2) / 6.0).clamp(11.0, 20.0);
            let name = crate::format::elide_to(&p, row.symbol.ticker(), &theme::semibold(big), r.width() - 10.0);
            p.text(
                r.center() - vec2(0.0, big * 0.55),
                Align2::CENTER_CENTER,
                name,
                theme::semibold(big),
                t.text,
            );
            p.text(
                r.center() + vec2(0.0, big * 0.6),
                Align2::CENTER_CENTER,
                metric.format(v),
                theme::regular((big * 0.75).max(10.5)),
                t.text2,
            );
        }
        if hot {
            let tip = format!(
                "{}\nWeight {:.1}%\n{}: {}",
                row.symbol.id,
                row.weight,
                metric.name(),
                metric.format(v)
            );
            if resp.clone().on_hover_text(tip).clicked() {
                open = Some(row.symbol.clone());
            }
        }
    }
    open
}

// ------------------------------------------------------------ editor

/// "New index" / "Edit index": a name, members (search, or a whole
/// watchlist) and the weighting.
pub struct IndexEditor {
    /// The index being edited (its ticker), or a new one.
    editing: Option<String>,
    name: String,
    members: Vec<Symbol>,
    weighting: Weighting,
    query: String,
    sent: String,
    seq: u64,
    pub results: Vec<Symbol>,
    selected: usize,
    confirm_delete: bool,
}

impl IndexEditor {
    pub fn new(def: Option<&IndexDef>) -> IndexEditor {
        IndexEditor {
            editing: def.map(|d| d.ticker.clone()),
            name: def.map(|d| d.name.clone()).unwrap_or_default(),
            members: def.map(|d| d.members.clone()).unwrap_or_default(),
            weighting: def.map(|d| d.weighting).unwrap_or_default(),
            query: String::new(),
            sent: String::new(),
            seq: 0,
            results: Vec::new(),
            selected: 0,
            confirm_delete: false,
        }
    }

    fn add(&mut self, s: Symbol) {
        if !self.members.iter().any(|m| m.id == s.id) {
            self.members.push(s);
        }
    }

    pub fn show(&mut self, ui: &mut Ui, app: &mut App, t: &Theme) -> Done {
        let mut done = Done::Keep;
        let title = if self.editing.is_some() {
            "Edit index"
        } else {
            "New index"
        };
        if frame(ui, t, title, 520.0, |ui| {
            let label = |ui: &mut Ui, s: &str| {
                ui.label(egui::RichText::new(s).font(theme::medium(12.0)).color(t.text2));
            };
            label(ui, "Name");
            let name = ui.add(
                w::field(&mut self.name)
                    .hint_text("e.g. Nifty Auto")
                    .desired_width(f32::INFINITY),
            );
            if self.editing.is_none() && self.name.is_empty() && self.members.is_empty() && !name.has_focus() {
                name.request_focus();
            }
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                label(ui, "Symbols");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    w::combo("index-from-watchlist")
                        .selected_text(format!("{}  Add a watchlist", ph::LIST_BULLETS))
                        .width(170.0)
                        .show_ui(ui, |ui| {
                            for wl in app.watchlists.clone() {
                                if w::option(ui, false, &wl.name).clicked() {
                                    for s in wl.symbols() {
                                        self.add(s);
                                    }
                                }
                            }
                        });
                });
            });
            // Search: type, arrows pick, Enter adds.
            let (up, down, enter) = ui.input_mut(|i| {
                (
                    i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp),
                    i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown),
                    i.consume_key(egui::Modifiers::NONE, egui::Key::Enter),
                )
            });
            let search = ui.add(
                w::field(&mut self.query)
                    .hint_text(format!("{}  Search symbols to add", ph::MAGNIFYING_GLASS))
                    .desired_width(f32::INFINITY),
            );
            let q = self.query.trim().to_string();
            if q != self.sent {
                self.sent = q.clone();
                self.selected = 0;
                if q.is_empty() {
                    self.results.clear();
                } else {
                    self.seq += 1;
                    app.search(app.search_seq + 1, &q);
                }
            }
            let shown: Vec<Symbol> = self.results.iter().take(6).cloned().collect();
            if down {
                self.selected = (self.selected + 1).min(shown.len().saturating_sub(1));
            }
            if up {
                self.selected = self.selected.saturating_sub(1);
            }
            if enter
                && search.has_focus()
                && let Some(s) = shown.get(self.selected)
            {
                self.add(s.clone());
                self.query.clear();
                search.request_focus();
            }
            if !q.is_empty() {
                ui.add_space(4.0);
                for (k, s) in shown.iter().enumerate() {
                    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::click());
                    let p = ui.painter();
                    let added = self.members.iter().any(|m| m.id == s.id);
                    if k == self.selected || resp.hovered() {
                        p.rect_filled(r, 6, t.hover_wash());
                    }
                    p.text(
                        r.left_center() + vec2(10.0, 0.0),
                        Align2::LEFT_CENTER,
                        s.ticker(),
                        theme::medium(12.5),
                        t.text,
                    );
                    let desc = crate::format::elide_to(p, &s.description, &theme::regular(12.0), r.width() - 230.0);
                    p.text(
                        r.left_center() + vec2(110.0, 0.0),
                        Align2::LEFT_CENTER,
                        desc,
                        theme::regular(12.0),
                        t.text2,
                    );
                    p.text(
                        r.right_center() - vec2(10.0, 0.0),
                        Align2::RIGHT_CENTER,
                        if added { ph::CHECK } else { ph::PLUS },
                        theme::regular(13.0),
                        if added { t.accent } else { t.text3 },
                    );
                    p.text(
                        r.right_center() - vec2(34.0, 0.0),
                        Align2::RIGHT_CENTER,
                        s.exchange(),
                        theme::regular(11.5),
                        t.text3,
                    );
                    if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                        self.add(s.clone());
                    }
                }
            }
            ui.add_space(8.0);
            // Members as chips.
            if self.members.is_empty() {
                ui.label(egui::RichText::new("No symbols yet").color(t.text3).size(12.5));
            } else {
                let mut remove = None;
                // Rows laid out by hand from the chips' widths (the
                // wrapping layout grew the dialog sideways instead).
                let wrap_w = ui.available_width();
                let font = theme::medium(12.0);
                let widths: Vec<f32> = self
                    .members
                    .iter()
                    .map(|m| {
                        ui.painter()
                            .layout_no_wrap(m.ticker().to_string(), font.clone(), t.text)
                            .size()
                            .x
                            + 36.0
                    })
                    .collect();
                let mut rows: Vec<Vec<usize>> = vec![Vec::new()];
                let mut x = 0.0;
                for (k, wd) in widths.iter().enumerate() {
                    if x + wd > wrap_w && !rows.last().is_some_and(|r| r.is_empty()) {
                        rows.push(Vec::new());
                        x = 0.0;
                    }
                    rows.last_mut().expect("a row").push(k);
                    x += wd + 6.0;
                }
                egui::ScrollArea::vertical()
                    .id_salt("index-members")
                    .max_height(120.0)
                    .show(ui, |ui| {
                        for row in rows {
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 6.0;
                                for k in row {
                                    let m = &self.members[k];
                                    let (r, resp) = ui.allocate_exact_size(vec2(widths[k], 26.0), Sense::hover());
                                    ui.painter().rect_filled(r, 6, t.hover);
                                    ui.painter().text(
                                        r.left_center() + vec2(8.0, 0.0),
                                        Align2::LEFT_CENTER,
                                        m.ticker(),
                                        font.clone(),
                                        t.text,
                                    );
                                    resp.on_hover_text(&m.id);
                                    let x =
                                        Rect::from_center_size(r.right_center() - vec2(13.0, 0.0), vec2(20.0, 20.0));
                                    let xr = ui
                                        .interact(x, ui.id().with(("index-chip-x", k)), Sense::click())
                                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                                        .on_hover_text("Remove");
                                    if xr.hovered() {
                                        ui.painter().rect_filled(x, 4, t.hover_wash());
                                    }
                                    ui.painter().text(
                                        x.center(),
                                        Align2::CENTER_CENTER,
                                        ph::X,
                                        theme::regular(12.0),
                                        if xr.hovered() { t.text } else { t.text3 },
                                    );
                                    if xr.clicked() {
                                        remove = Some(k);
                                    }
                                }
                            });
                            ui.add_space(2.0);
                        }
                    });
                if let Some(k) = remove {
                    self.members.remove(k);
                }
            }
            ui.add_space(12.0);
            label(ui, "Weighting");
            w::segmented(
                ui,
                t,
                &mut self.weighting,
                &[(Weighting::MarketCap, "Market cap"), (Weighting::Equal, "Equal")],
            );
            ui.add_space(16.0);
            ui.horizontal(|ui| {
                if let Some(ticker) = self.editing.clone() {
                    let label = if self.confirm_delete {
                        "Delete for good"
                    } else {
                        "Delete"
                    };
                    if w::button(ui, w::Kind::Danger, true, Some(ph::TRASH), label).clicked() {
                        if self.confirm_delete {
                            app.delete_index(&ticker);
                            done = Done::Close;
                        } else {
                            self.confirm_delete = true;
                        }
                    }
                }
                ui.add_space((ui.available_width() - 154.0).max(0.0));
                if w::secondary(ui, "Cancel").clicked() {
                    done = Done::Close;
                }
                let ok = !self.name.trim().is_empty() && !self.members.is_empty();
                let verb = if self.editing.is_some() { "Save" } else { "Create" };
                if w::button(ui, w::Kind::Primary, ok, None, verb).clicked() && ok {
                    app.save_index(self);
                    done = Done::Close;
                }
            });
        }) {
            done = Done::Close;
        }
        done
    }
}

impl App {
    fn save_index(&mut self, e: &IndexEditor) {
        let taken: Vec<String> = self
            .indices
            .defs
            .iter()
            .filter(|d| Some(&d.ticker) != e.editing.as_ref())
            .map(|d| d.ticker.clone())
            .collect();
        let pos = e
            .editing
            .as_ref()
            .and_then(|t| self.indices.defs.iter().position(|d| d.ticker == *t));
        let mut def = pos.map(|k| self.indices.defs[k].clone()).unwrap_or_default();
        let same = def.members == e.members && def.weighting == e.weighting;
        def.name = e.name.trim().to_string();
        if pos.is_none() {
            def.ticker = ticker_for(&def.name, &taken);
        }
        if !same {
            def.members = e.members.clone();
            def.weighting = e.weighting;
            def.units.clear();
        }
        let ticker = def.ticker.clone();
        match pos {
            Some(k) => self.indices.defs[k] = def,
            None => self.indices.defs.push(def),
        }
        self.indices.open = Some(ticker.clone());
        self.indices.chart_when_ready = Some(ticker);
        self.save_indices();
        self.layout.tab = Some(crate::shell::Tab::Indices);
        self.indices_want();
        self.indices_settle();
    }

    fn delete_index(&mut self, ticker: &str) {
        self.indices.defs.retain(|d| d.ticker != ticker);
        if self.indices.open.as_deref() == Some(ticker) {
            self.indices.open = self.indices.defs.first().map(|d| d.ticker.clone());
        }
        self.save_indices();
        self.subscribe_quotes();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bars(prices: &[f64]) -> Vec<Bar> {
        prices
            .iter()
            .enumerate()
            .map(|(i, p)| Bar {
                time: i as i64 * 86_400_000,
                open: *p,
                high: *p,
                low: *p,
                close: *p,
                volume: 0.0,
            })
            .collect()
    }

    #[test]
    fn market_cap_index_starts_at_its_base() {
        let a = bars(&[100.0, 110.0, 121.0]);
        let b = bars(&[50.0, 50.0, 25.0]);
        // A: 10 shares (cap 1000), B: 20 shares (cap 1000).
        let (units, t0, fell_back) = work_out_units(&[(&a, 10.0), (&b, 20.0)], Weighting::MarketCap, 1000.0).unwrap();
        assert_eq!(t0, 0);
        assert!(!fell_back);
        let s = index_series(&[&a, &b], &units);
        assert!((s[0].close - 1000.0).abs() < 1e-9);
        // Day 2: A +21% on half the weight, B -50% on the other half.
        assert!((s[2].close - (1210.0 + 500.0) / 2.0).abs() < 1e-9);
        // No share count for one: equal weights, and it says so.
        let (u2, _, fb) = work_out_units(&[(&a, 10.0), (&b, f64::NAN)], Weighting::MarketCap, 1000.0).unwrap();
        assert!(fb);
        assert!((u2[0] * 100.0 - u2[1] * 50.0).abs() < 1e-9);
    }

    #[test]
    fn formula_charts_through_the_hub() {
        let def = IndexDef {
            ticker: "T".into(),
            name: "T".into(),
            members: vec![Symbol::bare("NSE:A"), Symbol::bare("NSE:B")],
            units: vec![0.5, 0.000001],
            ..Default::default()
        };
        let f = def.formula().unwrap();
        let (e, legs) = opentern_data::expr::parse(&f).unwrap();
        assert_eq!(legs, ["NSE:A", "NSE:B"]);
        assert!((e.eval(&[10.0, 1_000_000.0]) - 6.0).abs() < 1e-9);
        assert_eq!(ticker_for("Nifty Auto", &["NIFTYAUTO".into()]), "NIFTYAUTO2");
    }

    #[test]
    fn scores_rank_lower_ratios_better() {
        let pe = [10.0, 20.0, 30.0, -5.0];
        assert_eq!(Metric::Pe.score(10.0, &pe), Some(1.0));
        assert_eq!(Metric::Pe.score(-5.0, &pe), Some(-1.0));
        assert_eq!(Metric::GrossMargin.score(30.0, &[10.0, 30.0]), Some(1.0));
        assert!(Metric::Ret1M.score(-2.0, &[4.0, -2.0]).unwrap() < 0.0);
    }

    #[test]
    fn treemap_fills_its_rect() {
        let r = Rect::from_min_size(pos2(0.0, 0.0), vec2(300.0, 200.0));
        let tiles = squarify(&[50.0, 25.0, 15.0, 10.0], r);
        let area: f32 = tiles.iter().map(|t| t.area()).sum();
        assert!((area - r.area()).abs() < 1.0);
        assert!(tiles.iter().all(|t| r.expand(0.01).contains_rect(*t)));
    }
}

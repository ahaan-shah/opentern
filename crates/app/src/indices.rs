//! Custom indices: a named basket of symbols ("Nifty Auto"), weighted the
//! way NSE weights its indices, charted like any symbol and measured on one
//! board: the index's own numbers on top, then every member's, as a table
//! or a treemap; and every index side by side (Compare).
//!
//! The method follows NSE's, except caps:
//! - **Free-float market cap.** Each member counts its public float
//!   (Yahoo's `floatShares`, promoters' and locked-in holdings left out),
//!   or all its shares, or equally; picked per index.
//! - **Base 1000** on the first day every member has a price.
//! - **A divisor that keeps the level continuous.** Weights hold for a
//!   segment of time; a rebalance (1 April and 1 October, NSE's schedule,
//!   when the app sees the date pass; or any edit to the members, or
//!   "Rebalance now") starts a new segment whose weights are scaled so the
//!   index stands exactly where the old weights left it.
//! - **Corporate actions.** Prices are split-adjusted after the fact; each
//!   segment keeps the prices it was set at and rescales its units when a
//!   split shows up as a changed past price.
//!
//! The segments are formulas (`u1*NSE:MARUTI + u2*NSE:M&M …`) registered
//! with the hub under `CUSTOM:<TICKER>` (`opentern_data::expr::set_customs`),
//! so the chart, quotes, indicators and alerts work on it unchanged.
//!
//! Members' daily bars come by `Request::Fetch` (seqs from `SEQ_BASE`), their
//! fundamentals by `Request::Fundamentals` (Yahoo, `opentern_data::fundamentals`).

use crate::app::App;
use crate::dialogs::{Done, Modal, frame};
use crate::icons::ph;
use crate::motion::lerp_color;
use crate::theme::{self, Theme};
use crate::widgets as w;
use egui::{Align2, Color32, Rect, Sense, Stroke, Ui, pos2, vec2};
use opentern_core::{AssetKind, Bar, Symbol, Timeframe};
use opentern_data::Request;
use opentern_data::fundamentals::Fundamentals;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap};
use std::time::{Duration, Instant};

/// Fetch seqs for members' bars: `SEQ_BASE + n`.
pub const SEQ_BASE: u64 = 1 << 46;
/// Daily bars per member: enough to warm up a 200-day average and to see
/// the prices each recent segment was set at.
const BARS: usize = 1500;
/// Members' data is fetched again when it's older than this.
const STALE: Duration = Duration::from_secs(15 * 60);
const SETTINGS_KEY: &str = "custom_indices";
/// A past price that moved more than this since a segment was set means
/// a split (or similar) was adjusted in.
const SPLIT_TOLERANCE: f64 = 0.02;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Weighting {
    /// Public float only, as NSE.
    #[default]
    FreeFloat,
    /// All shares outstanding.
    MarketCap,
    Equal,
}

impl Weighting {
    fn name(self) -> &'static str {
        match self {
            Weighting::FreeFloat => "Free-float market cap",
            Weighting::MarketCap => "Full market cap",
            Weighting::Equal => "Equal weight",
        }
    }
}

/// Weights in force from a time on.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Segment {
    /// First bar time it applies to (`i64::MIN` for the first segment,
    /// which also covers history before the index was made).
    pub from: i64,
    /// Member ids, parallel to `units`.
    pub members: Vec<String>,
    pub units: Vec<f64>,
    /// The bar whose closes set the units, and those closes (the split
    /// check compares against them).
    pub anchor_time: i64,
    pub anchors: Vec<f64>,
    /// When it was worked out (ms): rebalance dates after this are due.
    pub made: i64,
}

impl Segment {
    fn formula(&self) -> String {
        self.members
            .iter()
            .zip(&self.units)
            .map(|(m, u)| format!("{u}*{m}"))
            .collect::<Vec<_>>()
            .join(" + ")
    }
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
    pub segments: Vec<Segment>,
    /// Members or weighting changed (or "Rebalance now"): a new segment is
    /// due as soon as the data is in.
    pub pending: bool,
    /// Cap weighting was asked for but some member had no share count.
    pub equal_fallback: bool,
    /// Before segments (0.2.2's test builds): one set of units.
    #[serde(skip_serializing)]
    units: Vec<f64>,
    #[serde(skip_serializing)]
    base_time: i64,
}

impl Default for IndexDef {
    fn default() -> Self {
        IndexDef {
            ticker: String::new(),
            name: String::new(),
            members: Vec::new(),
            weighting: Weighting::FreeFloat,
            base: 1000.0,
            segments: Vec::new(),
            pending: false,
            equal_fallback: false,
            units: Vec::new(),
            base_time: 0,
        }
    }
}

impl IndexDef {
    pub fn id(&self) -> String {
        format!("{}{}", opentern_data::expr::CUSTOM, self.ticker)
    }

    /// It can be charted (it has weights, maybe not yet for new members).
    pub fn ready(&self) -> bool {
        !self.segments.is_empty()
    }

    /// The hub's segments.
    fn hub_segments(&self) -> Vec<(i64, String)> {
        self.segments.iter().map(|s| (s.from, s.formula())).collect()
    }

    /// Every symbol whose prices the index needs: today's members and the
    /// current segment's (a removed member still sets the level until the
    /// next segment starts).
    fn needs(&self) -> Vec<Symbol> {
        let mut out = self.members.clone();
        if let Some(last) = self.segments.last() {
            for id in &last.members {
                if !out.iter().any(|m| m.id == *id) {
                    out.push(Symbol {
                        timezone: opentern_data::expr::guess_tz(id),
                        ..Symbol::bare(id)
                    });
                }
            }
        }
        out
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

    /// Older saves had one set of units: that's the first segment.
    fn migrate(&mut self) {
        if self.segments.is_empty() && self.units.len() == self.members.len() && !self.units.is_empty() {
            self.segments.push(Segment {
                from: i64::MIN,
                members: self.members.iter().map(|m| m.id.clone()).collect(),
                units: std::mem::take(&mut self.units),
                anchor_time: self.base_time,
                anchors: Vec::new(),
                made: 0,
            });
        }
    }
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

/// The close at or before `t`.
fn close_at(bars: &[Bar], t: i64) -> f64 {
    crate::screener::close_at(bars, t).unwrap_or(f64::NAN)
}

/// The bar at exactly `t`.
fn bar_at(bars: &[Bar], t: i64) -> Option<&Bar> {
    bars.binary_search_by_key(&t, |b| b.time).ok().map(|i| &bars[i])
}

/// The segment in force at `t`.
fn seg_at(segs: &[Segment], t: i64) -> Option<&Segment> {
    let k = segs.partition_point(|s| s.from <= t);
    segs.get(k.saturating_sub(1))
}

/// Times every one of `ids` has a bar.
fn common_times(ids: &[String], bars: &HashMap<String, &[Bar]>) -> Vec<i64> {
    let mut sets = ids
        .iter()
        .map(|id| bars.get(id).map(|b| b.iter().map(|x| x.time).collect::<BTreeSet<_>>()));
    let Some(Some(mut all)) = sets.next() else {
        return Vec::new();
    };
    for s in sets {
        let Some(s) = s else { return Vec::new() };
        all.retain(|t| s.contains(t));
    }
    all.into_iter().collect()
}

/// Units so `Σ units × price` is `level` at `prices`: share counts for cap
/// weights, else 1 ÷ price (equal). Also says whether cap weights had to
/// fall back to equal (a member with no share count).
fn units_for(prices: &[f64], shares: &[f64], weighting: Weighting, level: f64) -> Option<(Vec<f64>, bool)> {
    if prices.iter().any(|p| !p.is_finite() || *p <= 0.0) {
        return None;
    }
    let caps = weighting != Weighting::Equal;
    let have = shares.iter().all(|s| s.is_finite() && *s > 0.0);
    let raw: Vec<f64> = if caps && have {
        shares.to_vec()
    } else {
        prices.iter().map(|p| 1.0 / p).collect()
    };
    let total: f64 = raw.iter().zip(prices).map(|(u, p)| u * p).sum();
    (total > 0.0).then(|| (raw.iter().map(|u| u * level / total).collect(), caps && !have))
}

/// The share count a weighting uses.
fn shares_for(w: Weighting, f: Option<&Fundamentals>) -> f64 {
    match (w, f) {
        (Weighting::Equal, _) | (_, None) => f64::NAN,
        (Weighting::MarketCap, Some(f)) => f.shares,
        // No float figure: all shares, rather than dropping to equal.
        (Weighting::FreeFloat, Some(f)) => {
            if f.float_shares.is_finite() {
                f.float_shares
            } else {
                f.shares
            }
        }
    }
}

/// The most recent NSE rebalance date (1 April, 1 October, UTC midnight)
/// after `after` and at or before `now`.
fn rebalance_due(after: i64, now: i64) -> Option<i64> {
    let year = |ms: i64| {
        jiff::Timestamp::from_millisecond(ms)
            .map(|t| t.to_zoned(jiff::tz::TimeZone::UTC).year())
            .unwrap_or(1970)
    };
    let mut best = None;
    for y in year(after.max(0))..=year(now) {
        for m in [4, 10] {
            let Ok(z) = jiff::civil::date(y, m, 1).to_zoned(jiff::tz::TimeZone::UTC) else {
                continue;
            };
            let d = z.timestamp().as_millisecond();
            if d > after && d <= now {
                best = Some(d);
            }
        }
    }
    best
}

/// Rescales a segment's units where a past price changed under it (a
/// split adjusted in after the fact). True if anything changed.
fn adjust_for_splits(seg: &mut Segment, bars: &HashMap<String, &[Bar]>) -> bool {
    let mut changed = false;
    if seg.anchors.len() != seg.members.len() {
        // Older saves: take today's view of the anchor prices.
        let now: Vec<f64> = seg
            .members
            .iter()
            .map(|id| bars.get(id).map_or(f64::NAN, |b| close_at(b, seg.anchor_time)))
            .collect();
        if now.iter().all(|p| p.is_finite()) {
            seg.anchors = now;
            changed = true;
        }
        return changed;
    }
    for (k, id) in seg.members.iter().enumerate() {
        let Some(b) = bars.get(id) else { continue };
        // Only when the bars reach back to the anchor.
        if b.first().is_none_or(|x| x.time > seg.anchor_time) {
            continue;
        }
        let now = close_at(b, seg.anchor_time);
        let then = seg.anchors[k];
        if now.is_finite() && now > 0.0 && then.is_finite() && (then / now - 1.0).abs() > SPLIT_TOLERANCE {
            seg.units[k] *= then / now;
            seg.anchors[k] = now;
            changed = true;
        }
    }
    changed
}

/// The index's daily bars from its members' bars, each day on the segment
/// in force (days some member of that segment didn't trade are skipped).
fn index_series(segs: &[Segment], bars: &HashMap<String, &[Bar]>) -> Vec<Bar> {
    let mut times = BTreeSet::new();
    for s in segs {
        for id in &s.members {
            if let Some(b) = bars.get(id) {
                times.extend(b.iter().map(|x| x.time));
            }
        }
    }
    let mut out = Vec::new();
    for t in times {
        let Some(s) = seg_at(segs, t) else { continue };
        let (mut o, mut h, mut l, mut c) = (0.0, 0.0, 0.0, 0.0);
        let mut all = true;
        for (id, u) in s.members.iter().zip(&s.units) {
            match bars.get(id).and_then(|b| bar_at(b, t)) {
                Some(b) => {
                    o += u * b.open;
                    h += u * b.high;
                    l += u * b.low;
                    c += u * b.close;
                }
                None => {
                    all = false;
                    break;
                }
            }
        }
        if all {
            out.push(Bar {
                time: t,
                open: o,
                high: h.max(o).max(c),
                low: l.min(o).min(c),
                close: c,
                volume: 0.0,
            });
        }
    }
    out
}

/// The index's level at the close of `t` on its segments.
fn level_at(segs: &[Segment], bars: &HashMap<String, &[Bar]>, t: i64) -> f64 {
    let Some(s) = seg_at(segs, t) else { return f64::NAN };
    s.members
        .iter()
        .zip(&s.units)
        .map(|(id, u)| u * bars.get(id).map_or(f64::NAN, |b| close_at(b, t)))
        .sum()
}

/// Works out an index's weights as far as its data allows: the first
/// segment (base 1000), a due rebalance or pending edit (a new segment,
/// level kept), and split rescaling. True if `def` changed.
/// `funds` holds every member whose fundamentals answered (`None`: an error).
fn settle(
    def: &mut IndexDef,
    bars: &HashMap<String, &[Bar]>,
    funds: &HashMap<String, Option<&Fundamentals>>,
    failed: &BTreeSet<String>,
    now: i64,
) -> bool {
    let mut changed = false;
    for s in &mut def.segments {
        changed |= adjust_for_splits(s, bars);
    }
    // A member whose prices can't be had (a renamed or delisted ticker) is
    // left out, rather than holding the whole index back.
    let ids: Vec<String> = def
        .members
        .iter()
        .map(|m| m.id.clone())
        .filter(|id| !failed.contains(id))
        .collect();
    if ids.is_empty() || ids.iter().any(|id| !bars.contains_key(id)) {
        return changed;
    }
    // Shares, once every member's fundamentals have answered (cap weights).
    if def.weighting != Weighting::Equal && ids.iter().any(|id| !funds.contains_key(id)) {
        return changed;
    }
    let shares: Vec<f64> = ids
        .iter()
        .map(|id| shares_for(def.weighting, funds.get(id).copied().flatten()))
        .collect();
    let times = common_times(&ids, bars);
    let Some(&first) = times.first() else { return changed };
    let due = def.segments.last().and_then(|s| rebalance_due(s.made, now));
    if def.segments.is_empty() {
        let p0: Vec<f64> = ids.iter().map(|id| close_at(bars[id], first)).collect();
        if let Some((units, fb)) = units_for(&p0, &shares, def.weighting, def.base) {
            def.segments.push(Segment {
                from: i64::MIN,
                members: ids,
                units,
                anchor_time: first,
                anchors: p0,
                made: now,
            });
            def.equal_fallback = fb;
            def.pending = false;
            changed = true;
        }
        return changed;
    }
    // Members in and out of the index changed (one came back, or failed).
    let regrouped = def.segments.last().is_some_and(|s| {
        let mut a = s.members.clone();
        let mut b = ids.clone();
        a.sort();
        b.sort();
        a != b
    });
    if !(def.pending || regrouped || due.is_some()) {
        return changed;
    }
    // The new weights start on `start`; they're set at the close before it,
    // where the old weights' level is kept.
    let start = match due {
        Some(d) if !def.pending => times.iter().copied().find(|t| *t >= d),
        _ => times.last().copied(),
    };
    let Some(start) = start else { return changed };
    // Re-editing on the day a segment started replaces that segment.
    while def.segments.len() > 1 && def.segments.last().is_some_and(|s| s.from >= start) {
        def.segments.pop();
    }
    let old: Vec<String> = def.segments.last().map(|s| s.members.clone()).unwrap_or_default();
    let both: Vec<String> = ids.iter().chain(&old).cloned().collect();
    let Some(&prev) = common_times(&both, bars).iter().rev().find(|t| **t < start) else {
        return changed;
    };
    let level = level_at(&def.segments, bars, prev);
    let prices: Vec<f64> = ids.iter().map(|id| close_at(bars[id], prev)).collect();
    if !level.is_finite() || level <= 0.0 {
        return changed;
    }
    let Some((units, fb)) = units_for(&prices, &shares, def.weighting, level) else {
        return changed;
    };
    def.segments.push(Segment {
        from: start,
        members: ids,
        units,
        anchor_time: prev,
        anchors: prices,
        made: now,
    });
    def.equal_fallback = fb;
    def.pending = false;
    true
}

/// Yahoo's gaps cleaned up: it reports 0 for margins it doesn't have
/// (banks have no gross or EBITDA margin), and ratios on near-zero
/// earnings run to nonsense. Those become "no value".
fn clean(f: &Fundamentals) -> Fundamentals {
    let mut g = f.clone();
    let nan = f64::NAN;
    if g.gross_margin == 0.0 {
        g.gross_margin = nan;
    }
    if g.ebitda_margin == 0.0 {
        g.ebitda_margin = nan;
    }
    if g.ebit_margin == 0.0 && g.pat_margin == 0.0 {
        g.ebit_margin = nan;
    }
    if g.ebitda == 0.0 || g.ebitda_margin.is_nan() {
        g.ebitda = nan;
    }
    if g.revenue <= 0.0 {
        g.revenue = nan;
    }
    if !(g.ev_ebitda > 0.0 && g.ev_ebitda <= 200.0) {
        g.ev_ebitda = nan;
    }
    if g.pe.abs() > 1000.0 {
        g.pe = nan;
    }
    g
}

/// The index's fundamentals from its members': totals over totals, and
/// margins weighted by revenue.
fn index_fundamentals(funds: &[Fundamentals], out: &mut HashMap<Metric, f64>) {
    let sum = |f: &dyn Fn(&Fundamentals) -> (f64, f64)| {
        let (mut a, mut b, mut n) = (0.0, 0.0, 0);
        for x in funds {
            let (p, q) = f(x);
            if p.is_finite() && q.is_finite() {
                a += p;
                b += q;
                n += 1;
            }
        }
        if n > 0 && b != 0.0 { a / b } else { f64::NAN }
    };
    out.insert(Metric::Pe, sum(&|f| (f.market_cap, f.net_income)));
    out.insert(Metric::EvEbitda, sum(&|f| (f.enterprise_value, f.ebitda)));
    let g = sum(&|f| (f.revenue, f.revenue / (1.0 + f.revenue_growth)));
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
        out.insert(m, sum(&|f| (get(f) * f.revenue, f.revenue)) * 100.0);
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

/// Everything the board shows for one index, worked out when data changes.
#[derive(Clone, Debug, Default)]
pub struct Board {
    pub rows: Vec<BoardRow>,
    pub index: HashMap<Metric, f64>,
    pub value: f64,
    pub change: f64,
    /// The members' market caps together (sizes indices in Compare's map).
    pub total_cap: f64,
    /// Members still loading.
    pub waiting: usize,
}

fn board_for(def: &IndexDef, data: &HashMap<String, MemberData>) -> Board {
    let tz = jiff::tz::TimeZone::get(&def.symbol().timezone).unwrap_or(jiff::tz::TimeZone::UTC);
    let mut board = Board {
        value: f64::NAN,
        change: f64::NAN,
        ..Default::default()
    };
    let mut funds: Vec<Fundamentals> = Vec::new();
    let bars = bars_of(data);
    let current = def.segments.last();
    let mut caps = Vec::new();
    for m in &def.members {
        let d = data.get(&m.id);
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
            Some(Ok(b)) if !b.is_empty() => {
                let n = b.len();
                row.price = b[n - 1].close;
                if n >= 2 && b[n - 2].close != 0.0 {
                    row.change = (b[n - 1].close / b[n - 2].close - 1.0) * 100.0;
                }
                price_measures(b, &tz, &mut row.values);
                if let Some(s) = current
                    && let Some(k) = s.members.iter().position(|x| *x == m.id)
                {
                    caps.push((board.rows.len(), s.units[k] * row.price));
                }
            }
            Some(Ok(_)) => row.error = Some("No prices came back.".into()),
            Some(Err(e)) => {
                row.error = Some(if e.contains("(404)") {
                    "No prices: the ticker may have been renamed or delisted. Left out of the index.".into()
                } else {
                    format!("{e} Left out of the index.")
                })
            }
            None => board.waiting += 1,
        }
        match d.and_then(|d| d.fund.as_ref()) {
            Some(Ok(f)) => {
                let f = clean(f);
                member_fundamentals(&f, &mut row.values);
                row.market_cap = f.market_cap;
                funds.push(f);
            }
            Some(Err(e)) if row.error.is_none() => row.error = Some(format!("Fundamentals: {e}")),
            None if d.and_then(|d| d.bars.as_ref()).is_some() => board.waiting += 1,
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
    board.total_cap = funds.iter().map(|f| f.market_cap).filter(|c| c.is_finite()).sum();
    if def.ready() {
        let series = index_series(&def.segments, &bars);
        if let Some(last) = series.last() {
            board.value = last.close;
            let n = series.len();
            if n >= 2 && series[n - 2].close != 0.0 {
                board.change = (last.close / series[n - 2].close - 1.0) * 100.0;
            }
        }
        price_measures(&series, &tz, &mut board.index);
    }
    index_fundamentals(&funds, &mut board.index);
    board
}

/// Every member's bars that arrived.
fn bars_of(data: &HashMap<String, MemberData>) -> HashMap<String, &[Bar]> {
    data.iter()
        .filter_map(|(id, d)| match &d.bars {
            Some(Ok(b)) if !b.is_empty() => Some((id.clone(), b.as_slice())),
            _ => None,
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

/// A score's text colour: the further from the middle, the stronger the
/// green or red; middling values stay a quiet grey.
fn tone(t: &Theme, s: Option<f32>) -> Color32 {
    match s {
        None => t.text3,
        Some(s) if s.abs() < 0.08 => t.text2,
        Some(s) => lerp_color(t.text2, if s > 0.0 { t.pos } else { t.neg }, 0.35 + 0.65 * s.abs()),
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum View {
    #[default]
    Table,
    Map,
}

/// What a click on the board opens.
#[derive(Clone, Debug)]
enum Click {
    Symbol(Symbol),
    Index(String),
}

/// One row of a metrics table: a member, or an index (Compare).
struct TRow {
    label: String,
    symbol: Option<Symbol>,
    click: Option<Click>,
    weight: f64,
    price: f64,
    change: f64,
    values: HashMap<Metric, f64>,
    error: Option<String>,
}

impl TRow {
    fn get(&self, m: Metric) -> f64 {
        self.values.get(&m).copied().unwrap_or(f64::NAN)
    }

    fn of(r: &BoardRow) -> TRow {
        TRow {
            label: r.symbol.ticker().to_string(),
            symbol: Some(r.symbol.clone()),
            click: Some(Click::Symbol(r.symbol.clone())),
            weight: r.weight,
            price: r.price,
            change: r.change,
            values: r.values.clone(),
            error: r.error.clone(),
        }
    }
}

pub struct Indices {
    pub defs: Vec<IndexDef>,
    /// The index on the board.
    pub open: Option<String>,
    /// Every index side by side instead of one.
    pub compare: bool,
    data: HashMap<String, MemberData>,
    seq: u64,
    pub view: View,
    /// What colours the treemap.
    pub metric: Metric,
    /// The table's order: a metric (or `None` = weight / name), descending?
    sort: (Option<Metric>, bool),
    version: u64,
    boards: HashMap<String, (u64, Board)>,
    /// Put this index on the chart once its weights are worked out.
    pub(crate) chart_when_ready: Option<String>,
}

impl Indices {
    pub fn load(store: &opentern_core::Store) -> Indices {
        let mut defs: Vec<IndexDef> = store
            .value(SETTINGS_KEY)
            .and_then(|j| serde_json::from_str(&j).ok())
            .unwrap_or_default();
        for d in &mut defs {
            d.migrate();
        }
        let ix = Indices {
            open: defs.first().map(|d| d.ticker.clone()),
            compare: false,
            defs,
            data: HashMap::new(),
            seq: 0,
            view: View::Table,
            metric: Metric::Ret1M,
            sort: (None, true),
            version: 0,
            boards: HashMap::new(),
            chart_when_ready: None,
        };
        ix.register();
        ix
    }

    /// Tells the hub every index's segments.
    fn register(&self) {
        opentern_data::expr::set_customs(
            self.defs
                .iter()
                .filter(|d| d.ready())
                .map(|d| (d.id(), d.hub_segments())),
        );
    }

    pub fn by_ticker(&self, t: &str) -> Option<&IndexDef> {
        self.defs.iter().find(|d| d.ticker == t)
    }

    /// The indices ready to chart (for live quotes).
    pub fn symbols(&self) -> Vec<Symbol> {
        self.defs.iter().filter(|d| d.ready()).map(|d| d.symbol()).collect()
    }

    /// An index's board, worked out again only when data changed.
    fn board(&mut self, ticker: &str) -> Option<Board> {
        let def = self.defs.iter().find(|d| d.ticker == ticker)?;
        if let Some((v, b)) = self.boards.get(ticker)
            && *v == self.version
        {
            return Some(b.clone());
        }
        let b = board_for(def, &self.data);
        self.boards.insert(ticker.to_string(), (self.version, b.clone()));
        Some(b)
    }
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

    /// Asks for whatever any index's members are missing (the list shows
    /// every index's value, Compare every board).
    fn indices_want(&mut self) {
        let need: Vec<Symbol> = self.indices.defs.iter().flat_map(|d| d.needs()).collect();
        for m in need {
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

    /// Works out weights wherever the data allows (`settle`), and charts
    /// see the new ones.
    fn indices_settle(&mut self) {
        let now = jiff::Timestamp::now().as_millisecond();
        let ix = &mut self.indices;
        let bars = bars_of(&ix.data);
        let funds: HashMap<String, Option<&Fundamentals>> = ix
            .data
            .iter()
            .filter_map(|(id, d)| d.fund.as_ref().map(|f| (id.clone(), f.as_ref().ok())))
            .collect();
        let failed: BTreeSet<String> = ix
            .data
            .iter()
            .filter(|(_, d)| matches!(&d.bars, Some(Err(_))) || matches!(&d.bars, Some(Ok(b)) if b.is_empty()))
            .map(|(id, _)| id.clone())
            .collect();
        let mut settled = Vec::new();
        for def in &mut ix.defs {
            if settle(def, &bars, &funds, &failed, now) {
                settled.push(def.clone());
            }
        }
        if !settled.is_empty() {
            self.save_indices();
            self.subscribe_quotes();
            let id = self.chart.symbol.id.clone();
            if let Some(def) = settled.iter().find(|d| d.id() == id) {
                self.open_symbol(def.symbol());
            }
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

    /// An index's value and day change: live when there's a quote (not in
    /// demo, whose quotes don't add up), else from its daily bars.
    fn index_now(&mut self, def: &IndexDef) -> Option<(f64, f64)> {
        if !self.demo
            && let Some((l, p)) = self.quotes.get(&def.id()).copied()
            && p != 0.0
        {
            return Some((l, (l / p - 1.0) * 100.0));
        }
        let b = self.indices.board(&def.ticker)?;
        (b.value.is_finite() && b.value > 0.0).then_some((b.value, b.change))
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
        let list_w = 210.0_f32.min(rect.width() * 0.3);
        let list = Rect::from_min_size(rect.min, vec2(list_w, rect.height()));
        let main = Rect::from_min_max(pos2(list.right() + 16.0, rect.top()), rect.max);
        ui.scope_builder(egui::UiBuilder::new().max_rect(list), |ui| self.indices_list(ui, t));
        ui.painter().vline(
            list.right() + 8.0,
            list.y_range(),
            Stroke::new(1.0, t.border.gamma_multiply(0.6)),
        );
        ui.scope_builder(egui::UiBuilder::new().max_rect(main), |ui| {
            ui.set_clip_rect(main.intersect(ui.clip_rect()));
            if self.indices.compare && self.indices.defs.len() >= 2 {
                self.indices_compare(ui, t);
            } else if self.index_def().is_some() {
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

    /// A card per index (name, value, day change), and Compare on top
    /// once there are two.
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
        ui.add_space(6.0);
        let defs = self.indices.defs.clone();
        let mut pick = None;
        let mut compare = false;
        egui::ScrollArea::vertical()
            .id_salt("indices-list")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 6.0;
                if defs.len() >= 2 {
                    let on = self.indices.compare;
                    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 40.0), Sense::click());
                    card(ui, t, r, on, resp.hovered());
                    let p = ui.painter();
                    p.text(
                        r.left_center() + vec2(12.0, 0.0),
                        Align2::LEFT_CENTER,
                        ph::SQUARES_FOUR,
                        theme::regular(15.0),
                        if on { t.accent } else { t.text2 },
                    );
                    p.text(
                        r.left_center() + vec2(34.0, 0.0),
                        Align2::LEFT_CENTER,
                        "Compare",
                        theme::medium(13.0),
                        t.text,
                    );
                    if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                        compare = true;
                    }
                }
                for d in &defs {
                    let on = !self.indices.compare && self.indices.open.as_deref() == Some(&d.ticker);
                    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 56.0), Sense::click());
                    card(ui, t, r, on, resp.hovered());
                    let now = self.index_now(d);
                    let p = ui.painter();
                    let name = crate::format::elide_to(p, &d.name, &theme::semibold(13.0), r.width() - 24.0);
                    p.text(
                        r.left_top() + vec2(12.0, 10.0),
                        Align2::LEFT_TOP,
                        name,
                        theme::semibold(13.0),
                        t.text,
                    );
                    let y = r.bottom() - 16.0;
                    match now {
                        Some((v, ch)) => {
                            let g = p.layout_no_wrap(crate::format::price(v, 2), theme::medium(12.5), t.text2);
                            let x = r.left() + 12.0;
                            let gw = g.size().x;
                            p.galley(pos2(x, y - g.size().y / 2.0), g, t.text2);
                            change_pill(p, t, pos2(x + gw + 8.0, y), ch);
                        }
                        None => {
                            p.text(
                                pos2(r.left() + 12.0, y),
                                Align2::LEFT_CENTER,
                                format!("{} symbols", d.members.len()),
                                theme::regular(12.0),
                                t.text3,
                            );
                        }
                    }
                    if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                        pick = Some(d.clone());
                    }
                }
            });
        if compare {
            self.indices.compare = true;
        }
        if let Some(d) = pick {
            self.indices.compare = false;
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
        let Some(board) = self.indices.board(&def.ticker) else {
            return;
        };
        let now = self.index_now(&def);
        let mut rebalance = false;
        // ---- header: name, value, change; view, rebalance and edit
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
            match now {
                Some((v, ch)) => {
                    ui.label(egui::RichText::new(crate::format::price(v, 2)).font(theme::medium(15.0)));
                    let (r, _) = ui.allocate_exact_size(vec2(pill_w(ui.painter(), ch), 22.0), Sense::hover());
                    change_pill(ui.painter(), t, r.left_center(), ch);
                }
                None if board.waiting > 0 || def.pending => {
                    ui.add(egui::Spinner::new().size(13.0).color(t.text3));
                }
                None => {}
            }
            let (icon, color) = if def.equal_fallback {
                (ph::WARNING_CIRCLE, t.warn)
            } else {
                (ph::INFO, t.text3)
            };
            ui.label(egui::RichText::new(icon).color(color))
                .on_hover_text(method_note(&def));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                if w::tool_button(ui, t, ph::PENCIL_SIMPLE, "Edit index", false, 28.0).clicked() {
                    self.modal = Some(Modal::Index(Box::new(IndexEditor::new(Some(&def)))));
                }
                if w::tool_button(
                    ui,
                    t,
                    ph::ARROWS_CLOCKWISE,
                    "Rebalance now: today's free float, with the level kept",
                    false,
                    28.0,
                )
                .clicked()
                {
                    rebalance = true;
                }
                view_switch(ui, t, &mut self.indices.view, &mut self.indices.metric);
            });
        });
        if rebalance && let Some(d) = self.indices.defs.iter_mut().find(|d| d.ticker == def.ticker) {
            d.pending = true;
            // Fresh share counts first.
            for m in &d.members {
                if let Some(x) = self.indices.data.get_mut(&m.id) {
                    x.asked = None;
                }
            }
            self.indices_want();
        }
        ui.add_space(8.0);
        kpi_strip(ui, t, &board.index);
        ui.add_space(10.0);
        let rows: Vec<TRow> = board.rows.iter().map(TRow::of).collect();
        let pinned = TRow {
            label: "Index".into(),
            symbol: None,
            click: def.ready().then(|| Click::Index(def.ticker.clone())),
            weight: f64::NAN,
            price: board.value,
            change: board.change,
            values: board.index.clone(),
            error: None,
        };
        let click = match self.indices.view {
            View::Table => metric_table(
                ui,
                t,
                &self.logos,
                "index",
                &rows,
                Some(&pinned),
                &mut self.indices.sort,
                true,
            ),
            View::Map => treemap(
                ui,
                t,
                &self.logos,
                &rows
                    .iter()
                    .filter(|r| r.weight.is_finite() && r.weight > 0.0)
                    .map(|r| (r, r.weight))
                    .collect::<Vec<_>>(),
                self.indices.metric,
                board.waiting > 0,
            ),
        };
        self.index_click(click);
    }

    /// Every index side by side: the same metrics, a row (or tile) each.
    fn indices_compare(&mut self, ui: &mut Ui, t: &Theme) {
        let defs = self.indices.defs.clone();
        let mut rows = Vec::new();
        let mut caps = Vec::new();
        let mut waiting = false;
        for d in &defs {
            let Some(b) = self.indices.board(&d.ticker) else {
                continue;
            };
            let now = self.index_now(d);
            waiting |= b.waiting > 0;
            caps.push(b.total_cap);
            rows.push(TRow {
                label: d.name.clone(),
                symbol: None,
                click: d.ready().then(|| Click::Index(d.ticker.clone())),
                weight: f64::NAN,
                price: now.map_or(f64::NAN, |n| n.0),
                change: now.map_or(f64::NAN, |n| n.1),
                values: b.index.clone(),
                error: None,
            });
        }
        ui.horizontal(|ui| {
            ui.set_height(32.0);
            ui.label(egui::RichText::new("Compare indices").font(theme::semibold(16.0)));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                view_switch(ui, t, &mut self.indices.view, &mut self.indices.metric);
            });
        });
        ui.add_space(10.0);
        let click = match self.indices.view {
            View::Table => metric_table(
                ui,
                t,
                &self.logos,
                "compare",
                &rows,
                None,
                &mut self.indices.sort,
                false,
            ),
            View::Map => {
                // Tiles by the members' market cap together; equal when a
                // cap is missing.
                let all = caps.iter().all(|c| *c > 0.0);
                let items: Vec<(&TRow, f64)> = rows
                    .iter()
                    .zip(&caps)
                    .map(|(r, c)| (r, if all { *c } else { 1.0 }))
                    .collect();
                treemap(ui, t, &self.logos, &items, self.indices.metric, waiting)
            }
        };
        self.index_click(click);
    }

    fn index_click(&mut self, click: Option<Click>) {
        match click {
            Some(Click::Symbol(s)) => self.open_symbol_synced(s),
            Some(Click::Index(ticker)) => {
                if let Some(d) = self.indices.by_ticker(&ticker).cloned() {
                    self.indices.compare = false;
                    self.indices.open = Some(ticker);
                    self.open_symbol_synced(d.symbol());
                }
            }
            None => {}
        }
    }
}

/// How the index is built, for the info icon.
fn method_note(def: &IndexDef) -> String {
    let date = |ms: i64| {
        jiff::Timestamp::from_millisecond(ms)
            .map(|t| t.to_zoned(jiff::tz::TimeZone::UTC).strftime("%-d %b %Y").to_string())
            .unwrap_or_default()
    };
    let mut s = format!("{}, base {}.", def.weighting.name(), def.base);
    if def.equal_fallback {
        s.push_str("\nWeighted equally for now: a member has no share count.");
    }
    if let Some(last) = def.segments.last() {
        if last.from != i64::MIN {
            s.push_str(&format!("\nLast rebalanced {}.", date(last.from)));
        }
        let now = jiff::Timestamp::now().as_millisecond();
        if let Some(next) = (1..=8)
            .map(|k| now + k * 46 * 86_400_000)
            .find_map(|t| rebalance_due(now, t))
        {
            s.push_str(&format!("\nNext rebalance {} (1 April and 1 October).", date(next)));
        }
    }
    s
}

/// A list card's background: quiet, an accent edge when it's the open one.
fn card(ui: &Ui, t: &Theme, r: Rect, on: bool, hover: bool) {
    let p = ui.painter();
    let fill = if on {
        lerp_color(t.card, t.accent, 0.08)
    } else if hover {
        lerp_color(t.card, t.text, 0.04)
    } else {
        t.card
    };
    let edge = if on {
        t.accent.gamma_multiply(0.7)
    } else {
        t.border.gamma_multiply(0.7)
    };
    p.rect(r, 10, fill, Stroke::new(1.0, edge), egui::StrokeKind::Inside);
}

fn pill_w(p: &egui::Painter, ch: f64) -> f32 {
    p.layout_no_wrap(crate::format::signed_pct(ch), theme::semibold(11.5), Color32::WHITE)
        .size()
        .x
        + 14.0
}

/// A day change as a small tinted pill, its left edge at `at`.
fn change_pill(p: &egui::Painter, t: &Theme, at: egui::Pos2, ch: f64) {
    if !ch.is_finite() {
        return;
    }
    let c = if ch >= 0.0 { t.pos } else { t.neg };
    let g = p.layout_no_wrap(crate::format::signed_pct(ch), theme::semibold(11.5), c);
    let r = Rect::from_min_size(pos2(at.x, at.y - 10.0), vec2(g.size().x + 14.0, 20.0));
    p.rect_filled(r, 6, lerp_color(t.card, c, 0.16));
    p.galley(r.center() - g.size() / 2.0, g, c);
}

/// Table | Map, and the metric the map colours by.
fn view_switch(ui: &mut Ui, t: &Theme, view: &mut View, metric: &mut Metric) {
    w::segmented(ui, t, view, &[(View::Table, "Table"), (View::Map, "Map")]);
    if *view == View::Map {
        w::combo("index-map-metric")
            .selected_text(metric.name())
            .width(150.0)
            .show_ui(ui, |ui| {
                for m in Metric::ALL {
                    w::option_value(ui, metric, m, m.name());
                }
            });
    }
}

/// The index's metrics as tiles: one row when they fit with their names
/// whole, else two.
fn kpi_strip(ui: &mut Ui, t: &Theme, values: &HashMap<Metric, f64>) {
    let gap = 8.0;
    let label_font = theme::regular(11.5);
    let widest = Metric::ALL
        .iter()
        .map(|m| {
            ui.painter()
                .layout_no_wrap(m.name().to_string(), label_font.clone(), t.text)
                .size()
                .x
        })
        .fold(0.0, f32::max);
    let avail = ui.available_width();
    let fits = |n: f32| (avail - gap * (n - 1.0)) / n >= widest + 26.0;
    let per_row = [12.0, 6.0, 4.0].into_iter().find(|n| fits(*n)).unwrap_or(3.0);
    let tile_w = ((avail - gap * (per_row - 1.0)) / per_row).floor();
    for row in Metric::ALL.chunks(per_row as usize) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for m in row {
                let v = values.get(m).copied().unwrap_or(f64::NAN);
                let (r, resp) = ui.allocate_exact_size(vec2(tile_w, 58.0), Sense::hover());
                let p = ui.painter();
                p.rect(
                    r,
                    10,
                    t.card,
                    Stroke::new(1.0, t.border.gamma_multiply(0.6)),
                    egui::StrokeKind::Inside,
                );
                let label = crate::format::elide_to(p, m.name(), &label_font, r.width() - 22.0);
                p.text(
                    r.left_top() + vec2(11.0, 10.0),
                    Align2::LEFT_TOP,
                    label,
                    label_font.clone(),
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
                    r.left_bottom() + vec2(11.0, -10.0),
                    Align2::LEFT_BOTTOM,
                    m.format(v),
                    theme::semibold(15.0),
                    c,
                );
                resp.on_hover_text(m.tip());
            }
        });
        ui.add_space(gap - ui.spacing().item_spacing.y);
    }
}

/// The metrics table: a lead column (logo and ticker, or an index's
/// name), weight (members only), price and day change, then each metric
/// under its group. Numbers are coloured by how they stand among the rows.
#[allow(clippy::too_many_arguments)]
fn metric_table(
    ui: &mut Ui,
    t: &Theme,
    logos: &crate::logos::Logos,
    salt: &str,
    rows: &[TRow],
    pinned: Option<&TRow>,
    sort: &mut (Option<Metric>, bool),
    weights: bool,
) -> Option<Click> {
    let mut click = None;
    let head_font = theme::medium(12.0);
    let cell_font = theme::regular(12.5);
    let measure = |s: &str| {
        ui.painter()
            .layout_no_wrap(s.to_string(), head_font.clone(), t.text)
            .size()
            .x
    };
    let lead_w = 170.0;
    let mut fixed: Vec<(&str, f32)> = Vec::new();
    if weights {
        fixed.push(("Weight", 96.0));
    }
    fixed.push((if weights { "Price" } else { "Value" }, 92.0));
    fixed.push(("1D", 78.0));
    let mut widths: Vec<f32> = Metric::ALL
        .iter()
        .map(|m| (measure(m.short()) + 26.0).max(76.0))
        .collect();
    let used: f32 = lead_w + fixed.iter().map(|f| f.1).sum::<f32>() + widths.iter().sum::<f32>();
    let spare = (ui.available_width() - used).max(0.0) / widths.len() as f32;
    for w in &mut widths {
        *w += spare.min(40.0);
    }
    let x0 = lead_w + fixed.iter().map(|f| f.1).sum::<f32>();
    let xs: Vec<f32> = widths
        .iter()
        .scan(x0, |x, w| {
            let at = *x;
            *x += w;
            Some(at)
        })
        .collect();
    let total = x0 + widths.iter().sum::<f32>();
    let row_h = 32.0;
    let hair = Stroke::new(1.0, t.border.gamma_multiply(0.45));
    let columns: HashMap<Metric, Vec<f64>> = Metric::ALL
        .iter()
        .map(|m| (*m, rows.iter().map(|r| r.get(*m)).collect()))
        .collect();
    let max_weight = rows
        .iter()
        .map(|r| r.weight)
        .filter(|w| w.is_finite())
        .fold(0.0, f64::max);
    // Where one group ends and the next starts (a faint rule down the table).
    let bounds: Vec<f32> = (1..Metric::ALL.len())
        .filter(|k| Metric::ALL[*k].group() != Metric::ALL[k - 1].group())
        .map(|k| xs[k])
        .chain(std::iter::once(x0))
        .collect();
    let mut order: Vec<&TRow> = rows.iter().collect();
    order.sort_by(|a, b| {
        let (x, y) = match sort.0 {
            Some(m) => (a.get(m), b.get(m)),
            None if weights => (a.weight, b.weight),
            None => return a.label.cmp(&b.label).reverse(),
        };
        match (x.is_finite(), y.is_finite()) {
            (true, true) => x.partial_cmp(&y).unwrap_or(std::cmp::Ordering::Equal),
            (true, false) => std::cmp::Ordering::Greater,
            (false, true) => std::cmp::Ordering::Less,
            _ => std::cmp::Ordering::Equal,
        }
    });
    if sort.1 {
        order.reverse();
    }
    egui::ScrollArea::horizontal().id_salt((salt, "h")).show(ui, |ui| {
        ui.set_min_width(total);
        // Group headers: a soft band over exactly their columns.
        let (band, _) = ui.allocate_exact_size(vec2(total, 24.0), Sense::hover());
        let p = ui.painter();
        let mut k = 0;
        while k < Metric::ALL.len() {
            let g = Metric::ALL[k].group();
            let n = Metric::ALL[k..].iter().take_while(|m| m.group() == g).count();
            let w: f32 = widths[k..k + n].iter().sum();
            let r = Rect::from_min_size(band.min + vec2(xs[k] + 3.0, 0.0), vec2(w - 6.0, 22.0));
            p.rect_filled(r, 6, lerp_color(t.bg, t.text, 0.05));
            let label = crate::format::elide_to(p, g, &theme::medium(11.5), r.width() - 12.0);
            p.text(r.center(), Align2::CENTER_CENTER, label, theme::medium(11.5), t.text2);
            k += n;
        }
        // Column titles; click to sort.
        let (head, _) = ui.allocate_exact_size(vec2(total, row_h), Sense::hover());
        let mut sorted = None;
        let mut title = |ui: &mut Ui, r: Rect, text: &str, key: Option<Metric>, tip: &str, left: bool| {
            let on = sort.0 == key;
            let c = if on { t.accent } else { t.text3 };
            let shown = crate::format::elide_to(ui.painter(), text, &head_font, r.width() - 16.0);
            let (pos, align) = if left {
                (r.left_center() + vec2(10.0, 0.0), Align2::LEFT_CENTER)
            } else {
                (r.right_center() - vec2(10.0, 0.0), Align2::RIGHT_CENTER)
            };
            ui.painter().text(pos, align, shown, head_font.clone(), c);
            if ui
                .interact(r, ui.id().with((salt, "head", text)), Sense::click())
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .on_hover_text(tip)
                .clicked()
            {
                sorted = Some(key);
            }
        };
        let lead_title = if weights { "Symbol" } else { "Index" };
        let lead_tip = if weights { "Sort by weight" } else { "Sort by name" };
        title(
            ui,
            Rect::from_min_size(head.min, vec2(lead_w, row_h)),
            lead_title,
            None,
            lead_tip,
            true,
        );
        let mut x = lead_w;
        for (name, wd) in &fixed {
            let r = Rect::from_min_size(head.min + vec2(x, 0.0), vec2(*wd, row_h));
            let tip = match *name {
                "Weight" => "Today's weight in the index",
                "1D" => "Change since the last close",
                _ => "",
            };
            if *name == "Weight" {
                title(ui, r, name, None, tip, false);
            } else {
                ui.painter().text(
                    r.right_center() - vec2(10.0, 0.0),
                    Align2::RIGHT_CENTER,
                    *name,
                    head_font.clone(),
                    t.text3,
                );
                if !tip.is_empty() {
                    ui.interact(r, ui.id().with((salt, "fixed", *name)), Sense::hover())
                        .on_hover_text(tip);
                }
            }
            x += wd;
        }
        for (k, m) in Metric::ALL.iter().enumerate() {
            let r = Rect::from_min_size(head.min + vec2(xs[k], 0.0), vec2(widths[k], row_h));
            title(ui, r, m.short(), Some(*m), &format!("{}: {}", m.name(), m.tip()), false);
        }
        if let Some(key) = sorted {
            *sort = (key, if sort.0 == key { !sort.1 } else { true });
        }
        ui.painter()
            .hline(head.x_range(), head.bottom(), Stroke::new(1.0, t.border));
        egui::ScrollArea::vertical()
            .id_salt((salt, "v"))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                let all: Vec<(&TRow, bool)> = pinned
                    .map(|p| (p, true))
                    .into_iter()
                    .chain(order.iter().map(|r| (*r, false)))
                    .collect();
                for (row, pin) in all {
                    let sense = if row.click.is_some() {
                        Sense::click()
                    } else {
                        Sense::hover()
                    };
                    let (r, resp) = ui.allocate_exact_size(vec2(total, row_h), sense);
                    let p = ui.painter();
                    if pin {
                        p.rect_filled(r, 6, lerp_color(t.bg, t.text, 0.05));
                    } else if resp.hovered() {
                        p.rect_filled(r, 6, t.hover_wash());
                    }
                    for b in &bounds {
                        p.vline(r.left() + b, r.y_range(), hair);
                    }
                    let strong = pin;
                    // Lead: logo and ticker, or the name.
                    let mut tx = r.left() + 10.0;
                    if let Some(s) = &row.symbol {
                        logos.paint(p, t, s, pos2(r.left() + 19.0, r.center().y), 9.0);
                        tx = r.left() + 36.0;
                    }
                    let lead_font = if strong || row.symbol.is_none() {
                        theme::semibold(12.5)
                    } else {
                        theme::medium(12.5)
                    };
                    let label = crate::format::elide_to(p, &row.label, &lead_font, r.left() + lead_w - tx - 8.0);
                    p.text(pos2(tx, r.center().y), Align2::LEFT_CENTER, label, lead_font, t.text);
                    let mut x = lead_w;
                    for (name, wd) in &fixed {
                        let cell = Rect::from_min_size(r.min + vec2(x, 0.0), vec2(*wd, row_h));
                        let right = cell.right_center() - vec2(10.0, 0.0);
                        match *name {
                            "Weight" if row.weight.is_finite() => {
                                // A small bar beside the number.
                                let track = Rect::from_min_size(cell.left_center() + vec2(10.0, -2.0), vec2(30.0, 4.0));
                                p.rect_filled(track, 2, t.hover_wash());
                                if max_weight > 0.0 {
                                    let fill = Rect::from_min_size(
                                        track.min,
                                        vec2(track.width() * (row.weight / max_weight) as f32, 4.0),
                                    );
                                    p.rect_filled(fill, 2, t.accent.gamma_multiply(0.7));
                                }
                                p.text(
                                    right,
                                    Align2::RIGHT_CENTER,
                                    format!("{:.1}%", row.weight),
                                    cell_font.clone(),
                                    t.text2,
                                );
                            }
                            "Price" | "Value" if row.price.is_finite() && row.price > 0.0 => {
                                let d = row.symbol.as_ref().map_or(2, |s| s.precision().max(2));
                                p.text(
                                    right,
                                    Align2::RIGHT_CENTER,
                                    crate::format::price(row.price, d),
                                    cell_font.clone(),
                                    t.text,
                                );
                            }
                            "1D" if row.change.is_finite() => {
                                let c = if row.change >= 0.0 { t.pos } else { t.neg };
                                p.text(
                                    right,
                                    Align2::RIGHT_CENTER,
                                    crate::format::signed_pct(row.change),
                                    cell_font.clone(),
                                    c,
                                );
                            }
                            _ => {}
                        }
                        x += wd;
                    }
                    if row.error.is_some() && row.values.is_empty() {
                        p.text(
                            r.left_center() + vec2(x0 + 10.0, 0.0),
                            Align2::LEFT_CENTER,
                            row.error.as_deref().unwrap_or_default(),
                            theme::regular(12.0),
                            t.text3,
                        );
                    } else {
                        for (k, m) in Metric::ALL.iter().enumerate() {
                            let v = row.get(*m);
                            let cell = Rect::from_min_size(r.min + vec2(xs[k], 0.0), vec2(widths[k], row_h));
                            let c = if strong {
                                if v.is_finite() { t.text } else { t.text3 }
                            } else {
                                tone(t, m.score(v, &columns[m]))
                            };
                            let f = if strong {
                                theme::semibold(12.5)
                            } else {
                                cell_font.clone()
                            };
                            p.text(
                                cell.right_center() - vec2(10.0, 0.0),
                                Align2::RIGHT_CENTER,
                                m.format(v),
                                f,
                                c,
                            );
                        }
                    }
                    if !pin {
                        p.hline(r.x_range(), r.bottom(), hair);
                    }
                    let tip = match (&row.error, &row.symbol) {
                        (Some(e), _) => Some(e.clone()),
                        (None, Some(s)) => Some(s.id.clone()),
                        _ => None,
                    };
                    let resp = match tip {
                        Some(tip) => resp.on_hover_text(tip),
                        None => resp,
                    };
                    if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                        click = row.click.clone();
                    }
                }
            });
    });
    click
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
                    app.logos.paint(p, t, s, r.left_center() + vec2(18.0, 0.0), 9.0);
                    p.text(
                        r.left_center() + vec2(34.0, 0.0),
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
                            + 58.0
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
                                    app.logos
                                        .paint(ui.painter(), t, m, r.left_center() + vec2(15.0, 0.0), 8.0);
                                    ui.painter().text(
                                        r.left_center() + vec2(29.0, 0.0),
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
                &[
                    (Weighting::FreeFloat, "Free float"),
                    (Weighting::MarketCap, "Full market cap"),
                    (Weighting::Equal, "Equal"),
                ],
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

/// Tiles sized by `items`' weights, coloured by how `metric` stands among
/// them.
fn treemap(
    ui: &mut Ui,
    t: &Theme,
    logos: &crate::logos::Logos,
    items: &[(&TRow, f64)],
    metric: Metric,
    waiting: bool,
) -> Option<Click> {
    let area = ui.available_rect_before_wrap();
    let resp = ui.allocate_rect(area, Sense::click());
    let mut items: Vec<&(&TRow, f64)> = items.iter().filter(|(_, w)| w.is_finite() && *w > 0.0).collect();
    items.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    if items.is_empty() {
        if waiting {
            ui.put(area, egui::Spinner::new());
        }
        return None;
    }
    let all: Vec<f64> = items.iter().map(|(r, _)| r.get(metric)).collect();
    let total: f64 = items.iter().map(|(_, w)| w).sum();
    let rects = squarify(&items.iter().map(|(_, w)| *w).collect::<Vec<_>>(), area);
    let p = ui.painter_at(area);
    let hover = resp.hover_pos();
    let mut click = None;
    for ((row, wgt), r) in items.iter().zip(rects) {
        let r = r.shrink(2.0);
        let v = row.get(metric);
        let fill = match metric.score(v, &all) {
            Some(s) => {
                let c = if s >= 0.0 { t.pos } else { t.neg };
                lerp_color(t.card, c, 0.12 + 0.4 * s.abs())
            }
            None => t.card,
        };
        p.rect_filled(r, 8, fill);
        let hot = hover.is_some_and(|h| r.contains(h));
        if hot {
            p.rect_stroke(r, 8, Stroke::new(1.5, t.text2), egui::StrokeKind::Inside);
        }
        if r.width() > 50.0 && r.height() > 34.0 {
            let big = (r.width().min(r.height() * 2.2) / 7.0).clamp(11.0, 19.0);
            let font = theme::semibold(big);
            let name = crate::format::elide_to(&p, &row.label, &font, r.width() - 14.0);
            p.text(
                r.center() - vec2(0.0, big * 0.55),
                Align2::CENTER_CENTER,
                name,
                font,
                t.text,
            );
            p.text(
                r.center() + vec2(0.0, big * 0.65),
                Align2::CENTER_CENTER,
                metric.format(v),
                theme::regular((big * 0.78).max(10.5)),
                t.text2,
            );
            if let Some(s) = &row.symbol
                && r.height() > 70.0
                && r.width() > 70.0
            {
                logos.paint(&p, t, s, r.left_top() + vec2(16.0, 16.0), 8.0);
            }
        }
        if hot {
            let tip = format!(
                "{}\n{:.1}% of the map\n{}: {}",
                row.symbol.as_ref().map_or(row.label.clone(), |s| s.id.clone()),
                wgt / total * 100.0,
                metric.name(),
                metric.format(v)
            );
            if resp.clone().on_hover_text(tip).clicked() {
                click = row.click.clone();
            }
        }
    }
    click
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
            // A new segment from today keeps the level (the divisor).
            def.pending = def.ready();
        }
        let ticker = def.ticker.clone();
        match pos {
            Some(k) => self.indices.defs[k] = def,
            None => self.indices.defs.push(def),
        }
        self.indices.compare = false;
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

    const DAY: i64 = 86_400_000;

    fn bars(prices: &[f64]) -> Vec<Bar> {
        prices
            .iter()
            .enumerate()
            .map(|(i, p)| Bar {
                time: i as i64 * DAY,
                open: *p,
                high: *p,
                low: *p,
                close: *p,
                volume: 0.0,
            })
            .collect()
    }

    fn fund(shares: f64, float: f64) -> Fundamentals {
        Fundamentals {
            shares,
            float_shares: float,
            ..Default::default()
        }
    }

    fn def(ids: &[&str], w: Weighting) -> IndexDef {
        IndexDef {
            ticker: "T".into(),
            name: "T".into(),
            members: ids.iter().map(|id| Symbol::bare(id)).collect(),
            weighting: w,
            ..Default::default()
        }
    }

    #[test]
    fn free_float_index_starts_at_its_base_and_keeps_its_level() {
        let (a, b, c) = (
            bars(&[100.0, 110.0, 121.0, 121.0]),
            bars(&[50.0, 50.0, 25.0, 25.0]),
            bars(&[10.0, 10.0, 10.0, 20.0]),
        );
        let map: HashMap<String, &[Bar]> = [("A", &a), ("B", &b), ("C", &c)]
            .into_iter()
            .map(|(k, v)| (k.to_string(), v.as_slice()))
            .collect();
        // A floats 10 of 40 shares (float cap 1000), B 20 of 20 (1000).
        let (fa, fb, fc) = (fund(40.0, 10.0), fund(20.0, 20.0), fund(100.0, 50.0));
        let funds: HashMap<String, Option<&Fundamentals>> = [("A", &fa), ("B", &fb), ("C", &fc)]
            .into_iter()
            .map(|(k, v)| (k.to_string(), Some(v)))
            .collect();
        let mut d = def(&["A", "B"], Weighting::FreeFloat);
        assert!(settle(&mut d, &map, &funds, &BTreeSet::new(), 0));
        let s = index_series(&d.segments, &map);
        assert!((s[0].close - 1000.0).abs() < 1e-9);
        // Day 2: A +21% on half the weight, B -50% on the other half.
        assert!((s[2].close - (1210.0 + 500.0) / 2.0).abs() < 1e-9);
        // Adding C on the last day: the level at the close before is kept.
        let before = s[2].close;
        d.members.push(Symbol::bare("C"));
        d.pending = true;
        assert!(settle(&mut d, &map, &funds, &BTreeSet::new(), 1));
        assert_eq!(d.segments.len(), 2);
        assert_eq!(d.segments[1].from, 3 * DAY);
        assert!((level_at(&d.segments, &map, 2 * DAY) - before).abs() < 1e-9);
        // C doubles on day 3; A and B are flat, so the index moves by C's weight.
        let s = index_series(&d.segments, &map);
        let wc = 50.0 * 10.0 / (10.0 * 121.0 + 20.0 * 25.0 + 50.0 * 10.0);
        assert!((s[3].close / s[2].close - (1.0 + wc)).abs() < 1e-9);
    }

    #[test]
    fn a_member_without_prices_is_left_out() {
        let a = bars(&[100.0, 110.0]);
        let map: HashMap<String, &[Bar]> = [("A".to_string(), a.as_slice())].into_iter().collect();
        let fa = fund(10.0, 10.0);
        let funds: HashMap<String, Option<&Fundamentals>> = [("A".to_string(), Some(&fa))].into_iter().collect();
        let failed: BTreeSet<String> = ["GONE".to_string()].into_iter().collect();
        let mut d = def(&["A", "GONE"], Weighting::FreeFloat);
        assert!(settle(&mut d, &map, &funds, &failed, 0));
        assert_eq!(d.segments[0].members, ["A"]);
        let s = index_series(&d.segments, &map);
        assert!((s[1].close - 1100.0).abs() < 1e-9);
    }

    #[test]
    fn a_split_adjusted_in_later_rescales_units() {
        let a = bars(&[100.0, 100.0]);
        let map: HashMap<String, &[Bar]> = [("A".to_string(), a.as_slice())].into_iter().collect();
        let mut seg = Segment {
            from: i64::MIN,
            members: vec!["A".into()],
            units: vec![5.0],
            anchor_time: 0,
            // Set when the price was 200, before a 2:1 split.
            anchors: vec![200.0],
            made: 0,
        };
        assert!(adjust_for_splits(&mut seg, &map));
        assert_eq!(seg.units[0], 10.0);
        assert_eq!(seg.anchors[0], 100.0);
        assert!(!adjust_for_splits(&mut seg, &map));
    }

    #[test]
    fn rebalances_on_nse_dates() {
        let ms = |y, m, d| {
            jiff::civil::date(y, m, d)
                .to_zoned(jiff::tz::TimeZone::UTC)
                .unwrap()
                .timestamp()
                .as_millisecond()
        };
        assert_eq!(rebalance_due(ms(2026, 4, 2), ms(2026, 9, 30)), None);
        assert_eq!(rebalance_due(ms(2026, 4, 2), ms(2026, 10, 29)), Some(ms(2026, 10, 1)));
        assert_eq!(rebalance_due(ms(2025, 1, 1), ms(2026, 5, 1)), Some(ms(2026, 4, 1)));
    }

    #[test]
    fn missing_numbers_are_not_zero() {
        let f = Fundamentals {
            gross_margin: 0.0,
            ebitda_margin: 0.0,
            ev_ebitda: 1028.5,
            ebit_margin: 0.3,
            pat_margin: 0.2,
            ..Default::default()
        };
        let g = clean(&f);
        assert!(g.gross_margin.is_nan() && g.ebitda_margin.is_nan() && g.ev_ebitda.is_nan());
        assert_eq!(g.ebit_margin, 0.3);
    }

    #[test]
    fn formula_charts_through_the_hub() {
        let seg = Segment {
            members: vec!["NSE:M&M".into(), "NSE:B".into()],
            units: vec![0.5, 0.000001],
            ..Default::default()
        };
        let (e, legs) = opentern_data::expr::parse(&seg.formula()).unwrap();
        assert_eq!(legs, ["NSE:M&M", "NSE:B"]);
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

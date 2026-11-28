//! A chart: one symbol and interval, its bars, the studies on it, its
//! panes and how it's scrolled and zoomed. Drawing and input live in
//! `paint.rs` and `ui.rs`; this file owns the data and its life cycle.

pub mod draw;
pub mod drawing_ui;
use crate::i18n::{msg, nt};
mod paint;
pub mod profile;
pub mod replay;
pub mod scale;
mod scripted;
pub mod style;
pub mod transform;
pub mod ui;

use jiff::tz::TimeZone;
use opentern_core::{Bar, Symbol, Timeframe};
use opentern_data::{Event, Status};
use opentern_series::study::{Inputs, Instance, Meta, Placement, sessions};
use serde::{Deserialize, Serialize};
pub use style::{ChartStyle, StudyLook};

/// Bars of empty space kept right of the last bar by default.
pub const RIGHT_MARGIN: f64 = 6.0;
/// How long prices may go without moving, while the market is open, before
/// the chart (`Chart::stale_for`) and the watchlist say so.
pub const STALE_AFTER: std::time::Duration = std::time::Duration::from_secs(5 * 60);

/// 00:00 UTC of a date, where daily bars are stamped.
pub fn utc_midnight(d: jiff::civil::Date) -> i64 {
    d.to_zoned(TimeZone::UTC)
        .map(|z| z.timestamp().as_millisecond())
        .unwrap_or(0)
}

/// The symbol's market is open now and was `STALE_AFTER` ago, so the
/// night's quiet doesn't count in the minute before the open's first trade.
pub fn steadily_open(symbol: &Symbol, now_ms: i64) -> bool {
    use opentern_core::calendar;
    let market = calendar::Market::of(symbol);
    let since = now_ms - STALE_AFTER.as_millis() as i64;
    // A rate published once a day is never "stale" between publications.
    if opentern_core::daily_only(&symbol.id) {
        return false;
    }
    market == calendar::Market::Crypto
        || (calendar::state(market, now_ms) == calendar::State::Open
            && calendar::state(market, since) == calendar::State::Open)
}

/// The FOMC's target range (from, to) on the last day before `end` (ms),
/// once the Fed funds rate has loaded this run.
pub fn fed_target(end: i64) -> Option<(f64, f64)> {
    let days = opentern_data::nyfed::targets();
    let i = days.partition_point(|d| d.time < end);
    days[..i].last()?.target
}

/// Pre-market amber, after-hours blue (as TradingView's sun and moon).
pub fn extended_color(t: &crate::theme::Theme, e: opentern_core::calendar::Extended) -> egui::Color32 {
    match e {
        opentern_core::calendar::Extended::Pre => t.warn,
        opentern_core::calendar::Extended::Post => egui::Color32::from_rgb(0x4C, 0x8D, 0xF6),
    }
}

/// A fresh id for a data session (charts and compared symbols), unique
/// for the life of the process, so late events from a closed one are
/// never mistaken for another's.
pub fn new_session_id() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(100);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// TradingView's colours for compared symbols, in order.
pub const COMPARE_COLORS: [u32; 8] = [
    0xFF9800, 0x9C27B0, 0x00BCD4, 0xE91E63, 0x4CAF50, 0x795548, 0x3F51B5, 0xFFEB3B,
];

/// Where a compared symbol goes, as in TradingView's compare dialog.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum CompareMode {
    /// On the main scale, which switches to percent.
    #[default]
    Percent,
    /// Over the chart, with its own price scale beside the main one.
    Scale,
    /// In its own pane below, as candles with their own scale.
    Pane,
}

impl CompareMode {
    pub fn label(self) -> &'static str {
        match self {
            CompareMode::Percent => nt!("Same % scale"),
            CompareMode::Scale => nt!("New price scale"),
            CompareMode::Pane => nt!("New pane"),
        }
    }
}

/// Another symbol drawn on the chart: as a percent comparison (the price
/// scale switches to percent, as on TradingView's "Compare"), with its own
/// price scale, or in its own pane.
pub struct Compare {
    pub symbol: Symbol,
    pub session: u64,
    pub bars: Vec<Bar>,
    pub status: Status,
    pub color: opentern_series::Rgba,
    pub mode: CompareMode,
    /// The pane it lives in (`CompareMode::Pane`), else 0.
    pub pane: u64,
    /// Its own scale's range (`CompareMode::Scale`).
    pub scale: PaneScale,
    /// Its own scale is logarithmic / labelled in percent.
    pub log: bool,
    pub pct: bool,
    pub visible: bool,
}

impl Compare {
    /// The close at or before `t` (an as-of join on time).
    pub fn close_at(&self, t: i64) -> Option<f64> {
        let i = match self.bars.binary_search_by_key(&t, |b| b.time) {
            Ok(i) => i,
            Err(0) => return None,
            Err(i) => i - 1,
        };
        Some(self.bars[i].close)
    }

    /// The bar at or before `t`.
    pub fn bar_at(&self, t: i64) -> Option<&Bar> {
        let i = match self.bars.binary_search_by_key(&t, |b| b.time) {
            Ok(i) => i,
            Err(0) => return None,
            Err(i) => i - 1,
        };
        self.bars.get(i)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SavedCompare {
    pub symbol: Symbol,
    pub color: opentern_series::Rgba,
    /// Before modes: true meant its own scale.
    #[serde(default)]
    pub overlay: bool,
    #[serde(default)]
    pub mode: Option<CompareMode>,
    #[serde(default)]
    pub pane: u64,
    #[serde(default)]
    pub log: bool,
    #[serde(default)]
    pub pct: bool,
    #[serde(default = "yes")]
    pub visible: bool,
}

pub struct StudyView {
    pub uid: u64,
    pub inst: Instance,
    pub look: StudyLook,
    pub visible: bool,
    pub pane: u64,
}

impl StudyView {
    pub fn meta(&self) -> &'static Meta {
        self.inst.meta
    }

    /// Visible, and set to show on this kind of interval (the Visibility tab).
    pub fn shown(&self, tf: Timeframe) -> bool {
        self.visible && self.look.intervals[style::unit_index(tf.unit)]
    }

    /// "RSI 14 close 14": the legend title.
    pub fn title(&self, with_inputs: bool) -> String {
        let s = self.inst.inputs.summary(self.meta());
        if with_inputs && !s.is_empty() {
            format!("{} {s}", self.meta().short)
        } else {
            self.meta().short.to_string()
        }
    }
}

/// Price-scale state of one pane.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PaneScale {
    pub auto: bool,
    pub lo: f64,
    pub hi: f64,
}

impl Default for PaneScale {
    fn default() -> Self {
        PaneScale {
            auto: true,
            lo: 0.0,
            hi: 1.0,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Pane {
    pub id: u64,
    pub weight: f32,
    #[serde(default)]
    pub scale: PaneScale,
}

/// Horizontal position and zoom.
#[derive(Clone, Copy, Debug)]
pub struct View {
    /// Pixels per bar.
    pub bar_w: f32,
    /// The (fractional) bar index at the right edge of the plot.
    pub right: f64,
}

impl Default for View {
    fn default() -> Self {
        View {
            bar_w: 8.0,
            right: RIGHT_MARGIN,
        }
    }
}

pub struct Chart {
    pub id: u64,
    pub symbol: Symbol,
    pub tf: Timeframe,
    pub bars: Vec<Bar>,
    sessions: Vec<bool>,
    pub status: Status,
    /// When the newest bar last changed (or a new one began): the legend
    /// warns when a live chart's prices stop moving while its market is
    /// open (`stale_for`).
    moved: Option<std::time::Instant>,
    newest: Option<Bar>,
    pub complete: bool,
    pub studies: Vec<StudyView>,
    pub panes: Vec<Pane>,
    pub style: ChartStyle,
    pub view: View,
    pub log: bool,
    pub percent: bool,
    next_uid: u64,
    asked_more: bool,
    /// A date range to fit once bars arrive (the range bar's 1D…All):
    /// days back, 0 for year to date, -1 for everything.
    pub fit_days: Option<i64>,
    /// The drawings on this symbol (saved per symbol, not per layout).
    pub drawings: Vec<draw::Drawing>,
    /// Bumped when drawings change, so they get saved.
    pub drawings_changed: u64,
    undo: Vec<Vec<draw::Drawing>>,
    redo: Vec<Vec<draw::Drawing>>,
    /// Bumped on every change worth saving.
    pub changed: u64,
    /// Bumped whenever the bars change (scripts re-run on it).
    pub data_version: u64,
    /// Script studies from a saved layout, for the app to start (they need
    /// their script hosts before they can be added).
    pub pending_scripts: Vec<SavedStudy>,
    /// A script's latest error, by study, shown in its legend row.
    pub study_errors: std::collections::HashMap<u64, String>,
    /// The symbol's timezone (sessions) and the display timezone.
    pub sym_tz: TimeZone,
    pub tz: TimeZone,
    /// A strategy's fills and trades, drawn over the candles.
    pub marks: Option<std::sync::Arc<TradeMarks>>,
    /// The paper account's position and working orders on this symbol.
    pub paper: Vec<PaperLine>,
    /// Active price alerts on this symbol, when Alert lines are on.
    pub alerts: Vec<AlertLine>,
    /// What `alerts` was built from (`App::alert_lines`).
    pub alerts_sig: u64,
    /// Dividends and splits (stocks), and whether prices are adjusted for
    /// dividends (TradingView's ADJ). `raw` keeps the unadjusted bars.
    pub corporate: Option<opentern_data::yahoo::Corporate>,
    pub adj: bool,
    /// Pre- and after-hours bars (US stocks, intraday), as TradingView's EXT.
    pub ext: bool,
    /// The symbol's pre-market or after-hours price (US stocks and funds),
    /// shown beside the regular one; it never moves the bars.
    pub extended: Option<opentern_data::yahoo::Ext>,
    /// A perpetual's funding rate (fraction per period) and next payment
    /// (ms), for the legend; asked once a minute (`App::poll_funding`).
    pub funding: Option<(String, f64, i64)>,
    pub funding_asked: Option<(String, std::time::Instant)>,
    raw: Vec<Bar>,
    /// The box size (Renko, Kagi, Point & figure) the bars were built with.
    pub brick: f64,
    /// The chart type's transform key the bars were built with.
    built_for: String,
    /// Other symbols compared or overlaid.
    pub compares: Vec<Compare>,
    /// Bar replay, while it's on (see `replay.rs`).
    pub replay: Option<replay::Replay>,
}

/// An active price alert drawn on the chart.
#[derive(Clone, Debug, PartialEq)]
pub struct AlertLine {
    pub id: i64,
    pub price: f64,
    /// Its name, or what it watches ("Price crossing 70000").
    pub label: String,
}

#[derive(Clone, Debug)]
pub struct PaperLine {
    pub price: f64,
    pub label: String,
    pub buy: bool,
    /// The position (solid) rather than an order (dashed).
    pub position: bool,
    /// The working order this line is (drag it to move the order).
    pub order: Option<u64>,
}

/// What a strategy shows on the chart, as on TradingView.
#[derive(Clone, Debug, Default)]
pub struct TradeMarks {
    pub fills: Vec<MarkFill>,
    /// Entry → exit of each closed trade: (entry bar, price, exit bar, price, won).
    pub links: Vec<(usize, f64, usize, f64, bool)>,
    /// The open position's average price and side (long = true).
    pub position: Option<(f64, bool)>,
}

#[derive(Clone, Debug)]
pub struct MarkFill {
    pub bar: usize,
    pub price: f64,
    pub buy: bool,
    /// "Long +1", "Close entry(s) order Long -1", …
    pub label: String,
}

/// What's saved of a chart: in the layout, and (studies only) in
/// indicator templates.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SavedStudy {
    pub id: String,
    pub inputs: Inputs,
    pub look: StudyLook,
    #[serde(default = "yes")]
    pub visible: bool,
    pub pane: u64,
    /// The study's id on the chart: a `Source` input linked to another
    /// indicator names it by this, so it's kept across a restart (0 in
    /// layouts from before links).
    #[serde(default)]
    pub uid: u64,
}

fn yes() -> bool {
    true
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChartState {
    pub version: u32,
    pub symbol: Symbol,
    pub tf: Timeframe,
    #[serde(default)]
    pub style: ChartStyle,
    #[serde(default)]
    pub studies: Vec<SavedStudy>,
    #[serde(default)]
    pub panes: Vec<Pane>,
    #[serde(default)]
    pub log: bool,
    #[serde(default)]
    pub adj: bool,
    #[serde(default)]
    pub ext: bool,
    #[serde(default)]
    pub compares: Vec<SavedCompare>,
    #[serde(default)]
    pub percent: bool,
}

/// An indicator template: the studies and how the panes were split.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StudyTemplate {
    pub version: u32,
    pub studies: Vec<SavedStudy>,
    pub panes: Vec<Pane>,
}

/// What "Save as default" keeps for an indicator.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StudyDefault {
    pub inputs: Inputs,
    pub look: StudyLook,
}

impl Chart {
    pub fn new(id: u64, symbol: Symbol, tf: Timeframe) -> Chart {
        let sym_tz = TimeZone::get(&symbol.timezone).unwrap_or(TimeZone::UTC);
        let mut c = Chart {
            id,
            tz: sym_tz.clone(),
            sym_tz,
            marks: None,
            paper: Vec::new(),
            alerts: Vec::new(),
            alerts_sig: 0,
            corporate: None,
            extended: None,
            funding: None,
            funding_asked: None,
            adj: false,
            ext: false,
            raw: Vec::new(),
            brick: 0.0,
            built_for: String::new(),
            compares: Vec::new(),
            replay: None,
            symbol,
            tf,
            bars: Vec::new(),
            sessions: Vec::new(),
            status: Status::Loading,
            moved: None,
            newest: None,
            complete: false,
            studies: Vec::new(),
            panes: vec![Pane {
                id: 0,
                weight: 3.0,
                scale: PaneScale::default(),
            }],
            style: ChartStyle::default(),
            view: View::default(),
            log: false,
            percent: false,
            next_uid: 1,
            asked_more: false,
            fit_days: None,
            drawings: Vec::new(),
            drawings_changed: 0,
            undo: Vec::new(),
            redo: Vec::new(),
            changed: 0,
            data_version: 0,
            pending_scripts: Vec::new(),
            study_errors: Default::default(),
        };
        c.apply_timezone();
        c
    }

    pub fn from_state(id: u64, s: ChartState) -> Chart {
        let mut c = Chart::new(id, s.symbol, s.tf);
        c.style = s.style;
        c.log = s.log;
        c.adj = s.adj;
        c.ext = s.ext;
        c.percent = s.percent;
        c.compares = s
            .compares
            .into_iter()
            .map(|x| {
                let mode = x.mode.unwrap_or(if x.overlay {
                    CompareMode::Scale
                } else {
                    CompareMode::Percent
                });
                Compare {
                    symbol: x.symbol,
                    session: 0,
                    bars: Vec::new(),
                    status: Status::Loading,
                    color: x.color,
                    mode,
                    pane: if mode == CompareMode::Pane { x.pane } else { 0 },
                    scale: PaneScale::default(),
                    log: x.log,
                    pct: x.pct,
                    visible: x.visible,
                }
            })
            .collect();
        if !s.panes.is_empty() && s.panes[0].id == 0 {
            c.panes = s.panes;
        }
        for k in 0..c.compares.len() {
            let p = c.compares[k].pane;
            if c.compares[k].mode == CompareMode::Pane && (p == 0 || !c.panes.iter().any(|x| x.id == p)) {
                let id = c.panes.iter().map(|x| x.id).max().unwrap_or(0) + 1;
                c.panes.push(Pane {
                    id,
                    weight: 1.0,
                    scale: PaneScale::default(),
                });
                c.compares[k].pane = id;
            }
        }
        // Fresh ids start above every saved one, so studies can take back
        // the ids their links name.
        c.next_uid = s
            .studies
            .iter()
            .map(|st| st.uid.max(st.pane))
            .chain(c.panes.iter().map(|p| p.id))
            .chain(c.compares.iter().map(|x| x.pane))
            .max()
            .unwrap_or(0)
            + 1;
        for st in s.studies {
            c.add_saved(&st);
        }
        // Drop panes nothing uses (a study that no longer exists); scripts
        // still to start keep theirs.
        let used: Vec<u64> = c
            .studies
            .iter()
            .map(|s| s.pane)
            .chain(c.pending_scripts.iter().map(|s| s.pane))
            .chain(c.compares.iter().map(|x| x.pane))
            .collect();
        c.panes.retain(|p| p.id == 0 || used.contains(&p.id));
        c.next_uid = c
            .studies
            .iter()
            .map(|s| s.uid)
            .chain(c.panes.iter().map(|p| p.id))
            .chain(c.compares.iter().map(|x| x.pane))
            .chain(c.pending_scripts.iter().map(|s| s.uid))
            .max()
            .unwrap_or(0)
            + 1;
        c.apply_timezone();
        c
    }

    pub fn state(&self) -> ChartState {
        ChartState {
            version: 1,
            symbol: self.symbol.clone(),
            tf: self.tf,
            style: self.style.clone(),
            studies: self.saved_studies(),
            panes: self.panes.clone(),
            log: self.log,
            adj: self.adj,
            ext: self.ext,
            percent: self.percent,
            compares: self
                .compares
                .iter()
                .map(|c| SavedCompare {
                    symbol: c.symbol.clone(),
                    color: c.color,
                    overlay: c.mode != CompareMode::Percent,
                    mode: Some(c.mode),
                    pane: c.pane,
                    log: c.log,
                    pct: c.pct,
                    visible: c.visible,
                })
                .collect(),
        }
    }

    pub fn owns_compare(&self, session: u64) -> bool {
        self.compares.iter().any(|c| c.session == session)
    }

    /// Data for a compared symbol.
    pub fn apply_compare(&mut self, e: Event) {
        let id = match &e {
            Event::Bars { chart, .. } | Event::Tick { chart, .. } | Event::Status { chart, .. } => *chart,
            _ => return,
        };
        let Some(c) = self.compares.iter_mut().find(|c| c.session == id) else {
            return;
        };
        if let Event::Bars { bars, .. } = &e
            && let Some(b) = bars.last()
        {
            c.symbol.retick(b.close);
        }
        if let Event::Tick { bar, .. } = &e {
            c.symbol.retick(bar.close);
        }
        match e {
            Event::Bars { bars, .. } => c.bars = bars,
            Event::Tick { bar, .. } => match c.bars.last_mut() {
                Some(last) if last.time == bar.time => *last = bar,
                Some(last) if last.time > bar.time => {}
                _ => c.bars.push(bar),
            },
            Event::Status { status, .. } => c.status = status,
            _ => {}
        }
    }

    /// Adds a symbol to compare (the app opens its data). Comparing puts
    /// the price scale in percent, as TradingView does.
    pub fn add_compare(&mut self, symbol: Symbol, mode: CompareMode) -> usize {
        let used: Vec<_> = self.compares.iter().map(|c| c.color).collect();
        let color = COMPARE_COLORS
            .iter()
            .map(|c| opentern_series::Rgba::hex(*c))
            .find(|c| !used.contains(c))
            .unwrap_or(opentern_series::Rgba::hex(COMPARE_COLORS[self.compares.len() % 8]));
        self.compares.push(Compare {
            symbol,
            session: 0,
            bars: Vec::new(),
            status: Status::Loading,
            color,
            mode: CompareMode::Percent,
            pane: 0,
            scale: PaneScale::default(),
            log: false,
            pct: false,
            visible: true,
        });
        let i = self.compares.len() - 1;
        self.set_compare_mode(i, mode);
        self.touch();
        i
    }

    /// Moves a compared symbol to the main % scale, its own scale or its
    /// own pane.
    pub fn set_compare_mode(&mut self, i: usize, mode: CompareMode) {
        let Some(c) = self.compares.get(i) else { return };
        let (old, old_pane) = (c.mode, c.pane);
        if old_pane != 0 && mode != CompareMode::Pane {
            self.panes.retain(|p| p.id != old_pane);
            self.compares[i].pane = 0;
        }
        if mode == CompareMode::Pane && old_pane == 0 {
            let id = self.new_uid();
            self.panes.push(Pane {
                id,
                weight: 1.0,
                scale: PaneScale::default(),
            });
            self.compares[i].pane = id;
        }
        self.compares[i].mode = mode;
        self.compares[i].scale = PaneScale::default();
        if mode == CompareMode::Percent {
            self.percent = true;
            self.log = false;
        } else if old == CompareMode::Percent && !self.compares.iter().any(|c| c.mode == CompareMode::Percent) {
            self.percent = false;
        }
        self.touch();
    }

    /// Removes a compared symbol; returns its data session to close.
    pub fn remove_compare(&mut self, i: usize) -> Option<u64> {
        if i >= self.compares.len() {
            return None;
        }
        let c = self.compares.remove(i);
        if c.pane != 0 {
            self.panes.retain(|p| p.id != c.pane);
        }
        if c.mode == CompareMode::Percent && !self.compares.iter().any(|c| c.mode == CompareMode::Percent) {
            self.percent = false;
        }
        self.touch();
        Some(c.session)
    }

    pub fn saved_studies(&self) -> Vec<SavedStudy> {
        self.studies
            .iter()
            .map(|s| SavedStudy {
                id: s.meta().id.to_string(),
                inputs: s.inst.inputs.clone(),
                look: s.look.clone(),
                visible: s.visible,
                pane: s.pane,
                uid: s.uid,
            })
            .collect()
    }

    pub fn apply_timezone(&mut self) {
        use style::TimeZoneChoice::*;
        self.tz = match &self.style.timezone {
            Exchange => self.sym_tz.clone(),
            Utc => TimeZone::UTC,
            Local => TimeZone::system(),
            // A zone this computer's database lacks falls back to UTC.
            Zone(z) => TimeZone::get(z).unwrap_or(TimeZone::UTC),
        };
    }

    pub fn touch(&mut self) {
        self.changed += 1;
    }

    /// Switches symbol and/or interval: the data starts over, the studies
    /// stay.
    pub fn reset_series(&mut self, symbol: Symbol, tf: Timeframe) {
        self.sym_tz = TimeZone::get(&symbol.timezone).unwrap_or(TimeZone::UTC);
        self.symbol = symbol;
        self.tf = tf;
        self.bars.clear();
        self.raw.clear();
        self.corporate = None;
        self.sessions.clear();
        self.status = Status::Loading;
        self.moved = None;
        self.newest = None;
        self.complete = false;
        self.asked_more = false;
        self.replay = None;
        for c in &mut self.compares {
            c.bars.clear();
            c.status = Status::Loading;
        }
        self.view = View {
            bar_w: self.view.bar_w,
            right: RIGHT_MARGIN,
        };
        for p in &mut self.panes {
            p.scale.auto = true;
        }
        self.apply_timezone();
        self.recompute_all();
        self.touch();
    }

    fn new_uid(&mut self) -> u64 {
        self.next_uid += 1;
        self.next_uid - 1
    }

    /// Adds a study with given inputs and look. Pane studies get a new pane
    /// below the others unless `pane` names an existing one.
    pub fn add_study(&mut self, meta: &'static Meta, inputs: &Inputs, look: StudyLook, pane: Option<u64>) -> u64 {
        self.add_study_as(meta, inputs, look, pane, 0)
    }

    /// [`Chart::add_study`], taking id `want` when it's free (a saved study
    /// whose id other studies' links name).
    pub fn add_study_as(
        &mut self,
        meta: &'static Meta,
        inputs: &Inputs,
        look: StudyLook,
        pane: Option<u64>,
        want: u64,
    ) -> u64 {
        let pane = match meta.placement {
            Placement::Overlay | Placement::Bottom => 0,
            Placement::Pane => match pane.filter(|p| *p != 0 && self.panes.iter().any(|x| x.id == *p)) {
                Some(p) => p,
                None => {
                    let id = self.new_uid();
                    self.panes.push(Pane {
                        id,
                        weight: 1.0,
                        scale: PaneScale::default(),
                    });
                    id
                }
            },
        };
        let uid = self.claim_uid(want);
        self.studies.push(StudyView {
            uid,
            inst: Instance::new(meta, inputs),
            look: look.fitted(meta),
            visible: true,
            pane,
        });
        self.added(uid);
        uid
    }

    /// Adds a study whose instance is already built (scripts), in `pane`
    /// (or a new pane for pane studies).
    #[cfg(test)]
    pub fn add_instance(&mut self, inst: Instance, look: StudyLook, pane: Option<u64>, visible: bool) -> u64 {
        self.add_instance_as(inst, look, pane, visible, 0)
    }

    /// [`Chart::add_instance`], taking id `want` when it's free (a saved
    /// script whose id other studies' links name).
    pub fn add_instance_as(
        &mut self,
        inst: Instance,
        look: StudyLook,
        pane: Option<u64>,
        visible: bool,
        want: u64,
    ) -> u64 {
        let meta = inst.meta;
        let pane = match meta.placement {
            Placement::Overlay | Placement::Bottom => 0,
            Placement::Pane => match pane.filter(|p| *p != 0) {
                Some(p) if self.panes.iter().any(|x| x.id == p) => p,
                Some(p) => {
                    self.panes.push(Pane {
                        id: p,
                        weight: 1.0,
                        scale: PaneScale::default(),
                    });
                    self.next_uid = self.next_uid.max(p + 1);
                    p
                }
                None => {
                    let id = self.new_uid();
                    self.panes.push(Pane {
                        id,
                        weight: 1.0,
                        scale: PaneScale::default(),
                    });
                    id
                }
            },
        };
        let uid = self.claim_uid(want);
        self.studies.push(StudyView {
            uid,
            look: look.fitted(meta),
            inst,
            visible,
            pane,
        });
        self.added(uid);
        uid
    }

    /// Id `want` if it's free (and not 0), else a fresh one.
    fn claim_uid(&mut self, want: u64) -> u64 {
        let used = |c: &Chart, id: u64| {
            c.studies.iter().any(|s| s.uid == id)
                || c.panes.iter().any(|p| p.id == id)
                || c.compares.iter().any(|x| x.pane == id)
        };
        if want != 0 && !used(self, want) {
            self.next_uid = self.next_uid.max(want + 1);
            return want;
        }
        self.new_uid()
    }

    /// A study was just added: run it (reading what it's linked to), then
    /// the studies already linked to its id (a layout restoring them).
    fn added(&mut self, uid: u64) {
        if let Some(i) = self.studies.iter().position(|s| s.uid == uid) {
            self.run_study(i, true);
        }
        self.refresh_readers(uid);
        self.touch();
    }

    /// The other studies' plots study `i`'s linked `Source` inputs read
    /// (empty values while the one named isn't on the chart).
    fn links_for(&self, i: usize) -> opentern_series::study::Links {
        self.studies[i]
            .inst
            .linked_inputs()
            .into_iter()
            .map(|(id, uid, plot)| {
                let vals = self
                    .studies
                    .iter()
                    .find(|s| s.uid == uid && s.uid != self.studies[i].uid)
                    .and_then(|s| s.inst.out.plots.get(plot))
                    .cloned()
                    .unwrap_or_default();
                (id.to_string(), std::sync::Arc::new(vals))
            })
            .collect()
    }

    /// The other indicators' lines study `uid` reads, by the text of the
    /// input that names each (`study:<uid>:<plot>`): what a Python script's
    /// `bars[name]` gives.
    pub fn linked_lines(&self, uid: u64) -> Vec<(String, std::sync::Arc<Vec<f64>>)> {
        let Some(i) = self.studies.iter().position(|s| s.uid == uid) else {
            return Vec::new();
        };
        let inputs = &self.studies[i].inst.inputs;
        self.links_for(i)
            .into_iter()
            .map(|(id, v)| (inputs.text(&id).to_string(), v))
            .collect()
    }

    /// Runs study `i` over the bars, from the first (`full`) or only the
    /// new and forming ones, with the plots it reads as they stand.
    fn run_study(&mut self, i: usize, full: bool) {
        let links = self.links_for(i);
        let (bars, sess, tf) = (&self.bars, &self.sessions, self.tf);
        let s = &mut self.studies[i];
        s.inst.set_links(links);
        if full {
            s.inst.recompute(bars, sess, tf);
        } else {
            s.inst.update(bars, sess, tf);
        }
    }

    /// The studies `uid` reads (its linked inputs' studies).
    fn reads(&self, i: usize) -> Vec<u64> {
        self.studies[i]
            .inst
            .linked_inputs()
            .into_iter()
            .map(|(_, u, _)| u)
            .collect()
    }

    /// Every study, ordered so each comes after the ones it reads. A loop
    /// (which the Source list never offers) is cut where it closes.
    fn study_order(&self) -> Vec<usize> {
        fn visit(c: &Chart, i: usize, state: &mut [u8], out: &mut Vec<usize>) {
            if state[i] != 0 {
                return;
            }
            state[i] = 1;
            for uid in c.reads(i) {
                if let Some(j) = c.studies.iter().position(|s| s.uid == uid) {
                    visit(c, j, state, out);
                }
            }
            state[i] = 2;
            out.push(i);
        }
        let mut state = vec![0u8; self.studies.len()];
        let mut out = Vec::with_capacity(self.studies.len());
        for i in 0..self.studies.len() {
            visit(self, i, &mut state, &mut out);
        }
        out
    }

    /// The studies that read `uid`, directly or through others.
    pub fn readers_of(&self, uid: u64) -> Vec<u64> {
        let mut found: Vec<u64> = Vec::new();
        let mut todo = vec![uid];
        while let Some(u) = todo.pop() {
            for (i, s) in self.studies.iter().enumerate() {
                if !found.contains(&s.uid) && s.uid != uid && self.reads(i).contains(&u) {
                    found.push(s.uid);
                    todo.push(s.uid);
                }
            }
        }
        found
    }

    /// Runs again, from the first bar, every study that reads `uid` (its
    /// values changed), in order.
    pub fn refresh_readers(&mut self, uid: u64) {
        let readers = self.readers_of(uid);
        if readers.is_empty() {
            return;
        }
        for i in self.study_order() {
            if readers.contains(&self.studies[i].uid) {
                self.run_study(i, true);
            }
        }
    }

    /// The legend's name for a study, a linked source named by the study
    /// and plot it reads ("RSI 14 EMA: EMA").
    pub fn study_title(&self, s: &StudyView, with_inputs: bool) -> String {
        if !with_inputs {
            return s.title(false);
        }
        let summary = s.inst.inputs.summary_with(s.meta(), |text| {
            let (uid, plot) = opentern_series::study::parse_link(text)?;
            Some(match self.studies.iter().find(|x| x.uid == uid) {
                Some(src) => format!(
                    "{}: {}",
                    src.meta().short,
                    src.meta().plots.get(plot).map_or("", |p| p.title)
                ),
                None => "—".into(),
            })
        });
        if summary.is_empty() {
            s.meta().short.to_string()
        } else {
            format!("{} {summary}", s.meta().short)
        }
    }

    /// Swaps a study's instance (a script whose outputs changed shape),
    /// keeping its place and as much of its look as still fits.
    pub fn replace_instance(&mut self, uid: u64, inst: Instance) {
        let Some(i) = self.studies.iter().position(|s| s.uid == uid) else {
            return;
        };
        let s = &mut self.studies[i];
        s.look = s.look.fitted(inst.meta);
        s.inst = inst;
        self.run_study(i, true);
        self.refresh_readers(uid);
    }

    pub fn bars_arc(&self) -> std::sync::Arc<Vec<Bar>> {
        std::sync::Arc::new(self.bars.clone())
    }

    fn add_saved(&mut self, s: &SavedStudy) {
        if ["py:", "pine:", "luau:", "js:", "ts:"]
            .iter()
            .any(|p| s.id.starts_with(p))
        {
            self.pending_scripts.push(s.clone());
            return;
        }
        let Some(meta) = opentern_series::builtins::by_id(&s.id) else {
            return;
        };
        let pane = if meta.placement == Placement::Pane {
            if !self.panes.iter().any(|p| p.id == s.pane) {
                self.panes.push(Pane {
                    id: s.pane.max(1),
                    weight: 1.0,
                    scale: PaneScale::default(),
                });
            }
            Some(s.pane)
        } else {
            None
        };
        self.next_uid = self.next_uid.max(s.pane + 1);
        let uid = self.add_study_as(meta, &s.inputs, s.look.clone(), pane, s.uid);
        if let Some(v) = self.studies.iter_mut().find(|v| v.uid == uid) {
            v.visible = s.visible;
        }
    }

    /// Replaces all studies with a template's.
    pub fn apply_study_template(&mut self, t: &StudyTemplate) {
        self.studies.clear();
        self.panes.retain(|p| p.id == 0);
        for p in &t.panes {
            if p.id != 0 {
                self.panes.push(p.clone());
            } else if let Some(main) = self.panes.first_mut() {
                main.weight = p.weight;
            }
        }
        for s in &t.studies {
            self.add_saved(s);
        }
        let used: Vec<u64> = self
            .studies
            .iter()
            .map(|s| s.pane)
            .chain(self.compares.iter().map(|c| c.pane))
            .collect();
        self.panes.retain(|p| p.id == 0 || used.contains(&p.id));
        self.touch();
    }

    pub fn remove_study(&mut self, uid: u64) {
        let Some(i) = self.studies.iter().position(|s| s.uid == uid) else {
            return;
        };
        let pane = self.studies.remove(i).pane;
        if pane != 0 && !self.studies.iter().any(|s| s.pane == pane) {
            self.panes.retain(|p| p.id != pane);
        }
        // Studies that read it go back to their own default source.
        let direct: Vec<(u64, Inputs)> = self
            .studies
            .iter()
            .filter_map(|s| {
                let linked = s.inst.linked_inputs();
                let ids: Vec<&str> = linked
                    .iter()
                    .filter(|(_, u, _)| *u == uid)
                    .map(|(id, ..)| *id)
                    .collect();
                if ids.is_empty() {
                    return None;
                }
                let mut inputs = s.inst.inputs.clone();
                for id in ids {
                    if let Some(spec) = s.meta().input(id) {
                        inputs.set(id, spec.default.clone());
                    }
                }
                Some((s.uid, inputs))
            })
            .collect();
        for (reader, inputs) in direct {
            self.set_inputs(reader, &inputs);
        }
        self.touch();
    }

    /// Takes a study off the chart to put it back in another place (a
    /// script that moved between the price pane and its own): unlike
    /// [`Chart::remove_study`], the studies linked to it keep their links,
    /// as it comes back with the same id.
    pub fn lift_study(&mut self, uid: u64) {
        if let Some(i) = self.studies.iter().position(|s| s.uid == uid) {
            let pane = self.studies.remove(i).pane;
            if pane != 0 && !self.studies.iter().any(|s| s.pane == pane) {
                self.panes.retain(|p| p.id != pane);
            }
        }
    }

    pub fn study_mut(&mut self, uid: u64) -> Option<&mut StudyView> {
        self.studies.iter_mut().find(|s| s.uid == uid)
    }

    /// New inputs for a study: it recomputes from the first bar.
    pub fn set_inputs(&mut self, uid: u64, inputs: &Inputs) {
        if let Some(i) = self.studies.iter().position(|s| s.uid == uid) {
            // Keeps how the study is built (a script's factory, or outputs
            // that come from a Python process).
            let was = self.studies[i].inst.levels();
            self.studies[i].inst = self.studies[i].inst.with_inputs(inputs);
            self.run_study(i, true);
            self.refresh_readers(uid);
            let s = &mut self.studies[i];
            // Levels follow the inputs they're made of, unless they've
            // been set by hand in the Style tab.
            for ((h, was), now) in s.look.hlines.iter_mut().zip(was).zip(s.inst.levels()) {
                if h.value == was.0 && now.0.is_finite() {
                    h.value = now.0;
                }
                if let (Some(a), Some(b)) = (was.1, now.1)
                    && h.color.0 == a
                {
                    h.color.0 = b;
                }
            }
        }
        self.touch();
    }

    fn recompute_all(&mut self) {
        for i in self.study_order() {
            self.run_study(i, true);
        }
    }

    pub fn last_index(&self) -> f64 {
        self.bars.len() as f64 - 1.0
    }

    /// Whether the view shows the latest bar (and should keep following it).
    pub fn following(&self) -> bool {
        self.view.right >= self.last_index()
    }

    /// The newest price on the chart, unless it's replaying the past.
    /// How long ago the newest bar last changed (None before any data).
    pub fn moved_ago(&self) -> Option<std::time::Duration> {
        self.moved.map(|m| m.elapsed())
    }

    pub fn last_price(&self) -> Option<f64> {
        (!self.replaying()).then(|| self.raw.last().map(|b| b.close)).flatten()
    }

    /// A quote that arrived after the chart's last update: moved into the
    /// forming bar, so the chart and the watchlist show the same price.
    /// Ignored once that bar's period is over (the quote may belong to the
    /// next one).
    pub fn take_quote(&mut self, price: f64, now_ms: i64) {
        let Some(&last) = self.raw.last() else { return };
        let span = self.tf.millis().unwrap_or(31 * 86_400_000);
        if self.replaying() || !price.is_finite() || price <= 0.0 || last.close == price || now_ms >= last.time + span {
            return;
        }
        let bar = Bar {
            close: price,
            high: last.high.max(price),
            low: last.low.min(price),
            ..last
        };
        self.apply(Event::Tick { chart: self.id, bar });
    }

    /// How long a live chart's prices have gone without moving while its
    /// market is open, once that's long enough to mention (5 minutes).
    /// Quiet symbols can go that long without a trade, so it's said as a
    /// fact, not an error.
    pub fn stale_for(&self, now_ms: i64) -> Option<std::time::Duration> {
        let quiet = self.moved?.elapsed();
        (self.status == Status::Live
            && !self.replaying()
            && quiet >= STALE_AFTER
            && steadily_open(&self.symbol, now_ms))
        .then_some(quiet)
    }

    /// This symbol's funding rate and next payment, when it's a perpetual.
    pub fn funding_now(&self) -> Option<(f64, i64)> {
        self.funding
            .as_ref()
            .filter(|(id, ..)| *id == self.symbol.id)
            .map(|(_, r, t)| (*r, *t))
    }

    /// Prices Yahoo is known to lack, said under the legend: BSE listings
    /// whose daily history is a single bar (most of them, Nov 2026), and
    /// an Indian special session (Muhurat trading) its bars skip, for a
    /// month after it, when this chart has no bars from it.
    pub fn data_gap(&self, now_ms: i64) -> Option<String> {
        use opentern_core::calendar::{self, Market};
        // Not waiting for `Live`: Yahoo's charts say Loading until the
        // first poll, 30 s after the history is already showing.
        if self.raw.is_empty() {
            return None;
        }
        let exchange = self.symbol.exchange();
        if !matches!(exchange, "NSE" | "BSE") || self.symbol.kind != opentern_core::AssetKind::Stock {
            return None;
        }
        if exchange == "BSE" && !self.tf.is_intraday() && self.raw.len() <= 2 {
            return Some(msg!(
                "Yahoo has no daily history for this BSE listing. NSE:{} has it.",
                self.symbol.ticker()
            ));
        }
        if !(self.tf.is_intraday() || self.tf.unit == opentern_core::Unit::Day) {
            return None;
        }
        let first = self.raw.first()?.time;
        let (day, open, close) = calendar::special_sessions(Market::India)
            .into_iter()
            .rfind(|(_, open, _)| *open <= now_ms)
            .filter(|(_, open, _)| now_ms - open < 30 * 86_400_000 && *open >= first)?;
        let midnight = crate::chart::utc_midnight(day);
        let has = if self.tf.is_intraday() {
            self.raw.iter().any(|b| b.time >= open && b.time < close)
        } else {
            self.raw.iter().any(|b| b.time == midnight)
        };
        if has {
            return None;
        }
        let name = calendar::special(Market::India, day).map_or(nt!("A special session"), |s| s.2);
        Some(msg!(
            "Yahoo's prices skip {name} on {}: it isn't on this chart.",
            day.strftime("%-d %b %Y")
        ))
    }

    /// Applies a data event meant for this chart.
    pub fn apply(&mut self, e: Event) {
        let newest = match &e {
            Event::Bars { bars, .. } => bars.last(),
            Event::Tick { bar, .. } => Some(bar),
            _ => None,
        };
        if let Some(b) = newest
            && self
                .newest
                .is_none_or(|n| b.time > n.time || (b.time == n.time && *b != n))
        {
            self.newest = Some(*b);
            self.moved = Some(std::time::Instant::now());
            // Hyperliquid's price step follows the price.
            self.symbol.retick(b.close);
        }
        if self.replay_absorb(&e) {
            return;
        }
        if matches!(e, Event::Bars { .. } | Event::Tick { .. }) {
            self.data_version += 1;
        }
        match e {
            Event::Bars { bars, complete, .. } => self.replace_bars(bars, complete),
            Event::Tick { bar, .. } => self.tick(bar),
            Event::Status { status, .. } => self.status = status,
            Event::Lot { min_qty, .. } => {
                self.symbol.min_qty = min_qty;
                self.touch();
            }
            Event::Corporate { corporate, .. } => {
                self.corporate = Some(corporate);
                if self.adj {
                    self.rebuild();
                }
            }
            _ => {}
        }
    }

    fn replace_bars(&mut self, bars: Vec<Bar>, complete: bool) {
        let was_empty = self.bars.is_empty();
        // Older history arrived at the front: keep the view where it was.
        let shift = match (self.bars.first(), bars.first()) {
            (Some(old), Some(_)) => bars.iter().position(|b| b.time == old.time).unwrap_or(0) as f64,
            _ => 0.0,
        };
        let following = self.following();
        self.raw = bars;
        self.bars = self.display();
        self.complete = complete;
        self.asked_more = false;
        self.sessions = sessions(&self.bars, self.tf, &self.sym_tz);
        self.recompute_all();
        if was_empty || following {
            self.view.right = self.last_index() + RIGHT_MARGIN;
        } else {
            self.view.right += shift;
        }
    }

    fn tick(&mut self, bar: Bar) {
        let following = self.following();
        // The newest bars are never scaled (no dividend comes after them).
        match self.raw.last_mut() {
            Some(last) if last.time == bar.time => *last = bar,
            Some(last) if last.time > bar.time => return,
            _ => self.raw.push(bar),
        }
        let bar = match self.style.transform(&[]) {
            style::Transform::None => bar,
            style::Transform::HeikinAshi => {
                let n = self.bars.len();
                let prev = match self.bars.last() {
                    Some(l) if l.time == bar.time => n.checked_sub(2).map(|i| self.bars[i]),
                    l => l.copied(),
                };
                transform::ha_bar(prev.as_ref(), &bar)
            }
            // Bricks, lines and columns: built again from all the bars.
            _ => {
                self.rebuild();
                if following {
                    self.view.right = self.last_index() + RIGHT_MARGIN;
                }
                return;
            }
        };
        match self.bars.last_mut() {
            Some(last) if last.time == bar.time => *last = bar,
            Some(last) if last.time > bar.time => return,
            _ => {
                self.bars.push(bar);
                let new_session = match self.bars.len() {
                    1 => true,
                    n => {
                        let day = |t: i64| {
                            jiff::Timestamp::from_millisecond(t)
                                .map(|x| x.to_zoned(self.sym_tz.clone()).date())
                                .ok()
                        };
                        !self.tf.is_intraday() || day(self.bars[n - 2].time) != day(bar.time)
                    }
                };
                self.sessions.push(new_session);
                if following {
                    self.view.right += 1.0;
                }
            }
        }
        for i in self.study_order() {
            self.run_study(i, false);
        }
    }

    /// The bars to show: raw (or adjusted for dividends), then changed by
    /// the chart type (Heikin Ashi, Renko, …).
    fn display(&mut self) -> Vec<Bar> {
        let base = self.adjusted();
        self.built_for = self.style.transform_key();
        let tr = self.style.transform(&base);
        self.brick = match tr {
            style::Transform::Renko(b) | style::Transform::Kagi(b) | style::Transform::PointFigure(b, _) => b,
            _ => 0.0,
        };
        match tr {
            style::Transform::None => base,
            style::Transform::HeikinAshi => transform::heikin_ashi(&base),
            style::Transform::Renko(b) => transform::renko(&base, b),
            style::Transform::LineBreak(n) => transform::line_break(&base, n),
            style::Transform::Kagi(b) => transform::kagi(&base, b),
            style::Transform::PointFigure(b, r) => transform::point_figure(&base, b, r),
        }
    }

    /// Builds the shown bars again from the raw ones (chart type or ADJ
    /// changed) and recomputes the studies.
    pub fn rebuild(&mut self) {
        let following = self.following();
        self.bars = self.display();
        self.sessions = sessions(&self.bars, self.tf, &self.sym_tz);
        self.recompute_all();
        self.data_version += 1;
        if following {
            self.view.right = self.last_index() + RIGHT_MARGIN;
        }
    }

    /// Rebuilds when the chart type's transform changed (after a style edit).
    pub fn style_changed(&mut self) {
        if self.style.transform_key() != self.built_for {
            self.rebuild();
        }
    }

    /// The bars to show: raw, or adjusted for dividends.
    fn adjusted(&self) -> Vec<Bar> {
        match (&self.corporate, self.adj) {
            (Some(c), true) if !c.dividends.is_empty() => {
                opentern_data::yahoo::dividend_adjust(&self.raw, &c.dividends, &self.sym_tz)
            }
            _ => self.raw.clone(),
        }
    }

    /// Whether EXT applies here: a US stock or fund, intraday.
    pub fn can_ext(&self) -> bool {
        self.tf.is_intraday()
            && opentern_core::calendar::Market::of(&self.symbol) == opentern_core::calendar::Market::Us
    }

    /// Whether pre- and after-hours bars are shown (and shaded).
    pub fn shows_ext(&self) -> bool {
        self.ext && self.can_ext()
    }

    /// The pre-market or after-hours price while those hours are on (an
    /// old one from earlier hours is dropped).
    pub fn extended_now(&self) -> Option<opentern_data::yahoo::Ext> {
        let e = self.extended?;
        (e.fresh(jiff::Timestamp::now().as_millisecond()) && !self.replaying()).then_some(e)
    }

    /// Turns dividend adjustment on or off.
    pub fn set_adj(&mut self, on: bool) {
        if self.adj == on {
            return;
        }
        self.adj = on;
        self.rebuild();
        self.touch();
    }

    /// True once per approach to the start of loaded history.
    pub fn wants_more(&mut self, first_visible: f64) -> bool {
        if self.complete || self.asked_more || self.bars.is_empty() || self.replaying() {
            return false;
        }
        if first_visible < 50.0 {
            self.asked_more = true;
            return true;
        }
        false
    }

    /// Decimals for prices on this chart.
    pub fn precision(&self) -> usize {
        self.style
            .precision
            .map(|p| p as usize)
            .unwrap_or_else(|| self.symbol.precision())
    }

    /// Fits `fit_days` of history to `width` pixels, once there are bars.
    /// Applied on every frame while the bars are still loading (cached ones
    /// show first, the network's follow), so the view never shows the new
    /// bars at the old zoom and then jumps; settled when loading is `done`.
    pub fn fit_range(&mut self, width: f32, done: bool) {
        let (Some(days), Some(last)) = (self.fit_days, self.bars.last()) else {
            return;
        };
        let from = match days {
            -1 => i64::MIN,
            0 => {
                let z = jiff::Timestamp::now().to_zoned(TimeZone::UTC);
                jiff::civil::date(z.year(), 1, 1)
                    .to_zoned(TimeZone::UTC)
                    .map(|d| d.timestamp().as_millisecond())
                    .unwrap_or(i64::MIN)
            }
            d => last.time - d * 86_400_000,
        };
        let count = self.bars.iter().filter(|b| b.time >= from).count().max(10) as f32;
        self.view.bar_w = (width / (count + RIGHT_MARGIN as f32)).clamp(0.4, 80.0);
        self.view.right = self.last_index() + RIGHT_MARGIN;
        for p in &mut self.panes {
            p.scale.auto = true;
        }
        if done {
            self.fit_days = None;
        }
    }

    pub fn next_drawing_id(&self) -> u64 {
        self.drawings.iter().map(|d| d.id).max().unwrap_or(0) + 1
    }

    /// Remembers the drawings as they are, before a change (for undo).
    pub fn push_undo(&mut self) {
        self.undo.push(self.drawings.clone());
        if self.undo.len() > 100 {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    pub fn undo(&mut self) -> bool {
        let Some(prev) = self.undo.pop() else { return false };
        self.redo.push(std::mem::replace(&mut self.drawings, prev));
        self.drawings_changed += 1;
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(next) = self.redo.pop() else { return false };
        self.undo.push(std::mem::replace(&mut self.drawings, next));
        self.drawings_changed += 1;
        true
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Swaps in another symbol's drawings (history doesn't carry over).
    pub fn set_drawings(&mut self, list: Vec<draw::Drawing>) {
        self.drawings = list;
        self.undo.clear();
        self.redo.clear();
    }

    /// Scrolls to the latest bar and puts every scale back on auto.
    pub fn reset_view(&mut self) {
        self.view.right = self.last_index() + RIGHT_MARGIN;
        self.view.bar_w = 8.0;
        for p in &mut self.panes {
            p.scale.auto = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use opentern_series::builtins::by_id;

    fn bars(n: usize, start: i64) -> Vec<Bar> {
        (0..n)
            .map(|i| {
                let x = (start + i as i64) as f64;
                let c = 100.0 + (x * 0.1).sin() * 5.0;
                Bar {
                    time: (start + i as i64) * 3_600_000,
                    open: c - 0.5,
                    high: c + 1.0,
                    low: c - 1.0,
                    close: c,
                    volume: 10.0,
                }
            })
            .collect()
    }

    fn chart() -> Chart {
        let sym = opentern_core::starter_watchlist().remove(0);
        Chart::new(1, sym, Timeframe::parse("1h").unwrap())
    }

    /// A range button's fit holds from the first (cached) bars while the
    /// rest load, and stops once they're in: the view never shows the new
    /// bars at the old zoom and then jumps.
    #[test]
    fn a_range_fit_holds_while_bars_load() {
        let mut c = chart();
        c.view.bar_w = 40.0;
        c.fit_days = Some(10);
        c.status = Status::Loading;
        // Cached bars: 5 days of hourly bars.
        c.bars = bars(120, 0);
        c.fit_range(1000.0, false);
        let early = c.view.bar_w;
        assert!(early < 10.0, "fitted at once, not at the old zoom: {early}");
        assert!(c.fit_days.is_some(), "still pending while loading");
        // The network's bars: 20 days. The fit follows them.
        c.bars = bars(480, 0);
        c.fit_range(1000.0, false);
        assert!(c.view.bar_w < early, "{} vs {early}", c.view.bar_w);
        c.status = Status::Live;
        c.fit_range(1000.0, true);
        assert!(c.fit_days.is_none(), "done once loaded");
        // The user's own zoom afterwards stays.
        c.view.bar_w = 30.0;
        c.fit_range(1000.0, true);
        assert_eq!(c.view.bar_w, 30.0);
    }

    // ---- indicator on indicator (linked Source inputs)
    use opentern_series::study::Value;

    /// SMA worked out here, not with the app's code: na until a whole
    /// window of numbers.
    fn sma_ref(x: &[f64], n: usize) -> Vec<f64> {
        (0..x.len())
            .map(|i| {
                if i + 1 < n {
                    return f64::NAN;
                }
                let w = &x[i + 1 - n..=i];
                if w.iter().any(|v| !v.is_finite()) {
                    f64::NAN
                } else {
                    w.iter().sum::<f64>() / n as f64
                }
            })
            .collect()
    }

    /// Wilder's RSI worked out here, as Pine defines it: changes, then an
    /// RMA seeded with the SMA of the first `n` changes.
    fn rsi_ref(x: &[f64], n: usize) -> Vec<f64> {
        let ch: Vec<f64> = (0..x.len())
            .map(|i| if i == 0 { f64::NAN } else { x[i] - x[i - 1] })
            .collect();
        let rma = |v: Vec<f64>| -> Vec<f64> {
            let mut out = vec![f64::NAN; v.len()];
            let a = 1.0 / n as f64;
            for i in 0..v.len() {
                let prev = if i > 0 { out[i - 1] } else { f64::NAN };
                out[i] = if prev.is_finite() {
                    a * v[i] + (1.0 - a) * prev
                } else {
                    sma_ref(&v[..=i], n).last().copied().unwrap_or(f64::NAN)
                };
            }
            out
        };
        let up = rma(ch
            .iter()
            .map(|c| if c.is_finite() { c.max(0.0) } else { f64::NAN })
            .collect());
        let dn = rma(ch
            .iter()
            .map(|c| if c.is_finite() { (-c).max(0.0) } else { f64::NAN })
            .collect());
        up.iter()
            .zip(&dn)
            .map(|(u, d)| {
                if !u.is_finite() || !d.is_finite() {
                    f64::NAN
                } else if *d == 0.0 {
                    100.0
                } else if *u == 0.0 {
                    0.0
                } else {
                    100.0 - 100.0 / (1.0 + u / d)
                }
            })
            .collect()
    }

    fn close_enough(got: &[f64], want: &[f64], what: &str) -> usize {
        assert_eq!(got.len(), want.len(), "{what}: lengths");
        let mut n = 0;
        for i in 0..got.len() {
            let (a, b) = (got[i], want[i]);
            if b.is_finite() {
                assert!((a - b).abs() < 1e-9, "{what}, bar {i}: {a} vs {b}");
                n += 1;
            } else {
                assert!(!a.is_finite(), "{what}, bar {i}: {a} where na");
            }
        }
        n
    }

    fn loaded(n: usize) -> Chart {
        let mut c = chart();
        c.apply(Event::Bars {
            chart: 1,
            bars: bars(n, 0),
            complete: true,
        });
        c
    }

    fn builtin(c: &mut Chart, id: &str, set: &[(&str, Value)]) -> u64 {
        let meta = by_id(id).unwrap();
        let mut inputs = meta.default_inputs();
        for (k, v) in set {
            inputs.set(k, v.clone());
        }
        c.add_study(meta, &inputs, StudyLook::defaults(meta), None)
    }

    fn pine(c: &mut Chart, src: &str, set: &[(&str, Value)]) -> u64 {
        let script = opentern_pine::compile(src, "t.pine", opentern_pine::Env::default()).unwrap();
        let mut inputs = script.meta().default_inputs();
        for (k, v) in set {
            inputs.set(k, v.clone());
        }
        let inst = Instance::with_factory(script.meta(), &inputs, script.factory());
        c.add_instance(inst, StudyLook::defaults(script.meta()), None, true)
    }

    fn plot(c: &Chart, uid: u64, k: usize) -> Vec<f64> {
        c.studies.iter().find(|s| s.uid == uid).unwrap().inst.out.plots[k].clone()
    }

    fn link(uid: u64, plot: usize) -> Value {
        Value::Text(opentern_series::study::link_key(uid, plot))
    }

    /// The built-in RSI on the built-in EMA (TradingView's "RSI of an EMA"):
    /// the RSI's numbers are Wilder's RSI of the EMA's values.
    #[test]
    fn a_builtin_reads_another_builtins_plot() {
        let mut c = loaded(300);
        let ema = builtin(&mut c, "ema", &[("length", Value::Int(9))]);
        let rsi = builtin(&mut c, "rsi", &[("length", Value::Int(14)), ("source", link(ema, 0))]);
        let want = rsi_ref(&plot(&c, ema, 0), 14);
        assert!(close_enough(&plot(&c, rsi, 0), &want, "RSI of EMA") > 250);
        // Not just the RSI of the close.
        let closes: Vec<f64> = c.bars.iter().map(|b| b.close).collect();
        let plain = rsi_ref(&closes, 14);
        assert!((plot(&c, rsi, 0)[200] - plain[200]).abs() > 1e-6);
        // The legend names what it reads.
        let s = c.studies.iter().find(|s| s.uid == rsi).unwrap();
        assert!(c.study_title(s, true).contains("EMA: "), "{}", c.study_title(s, true));
    }

    /// A Pine script's input.source on a built-in's plot, history included
    /// (`s[2]`), and a built-in on a Pine script's plot.
    #[test]
    fn pine_and_builtins_read_each_other() {
        let mut c = loaded(200);
        let ema = builtin(&mut c, "ema", &[("length", Value::Int(10))]);
        let script = pine(
            &mut c,
            "//@version=6\nindicator(\"Reader\")\ns = input.source(close, \"Src\")\nplot(ta.sma(s, 5), \"Avg\")\nplot(s[2], \"Back\")\nplot(s * 2, \"Double\")",
            &[("src", link(ema, 0))],
        );
        let e = plot(&c, ema, 0);
        assert!(close_enough(&plot(&c, script, 0), &sma_ref(&e, 5), "ta.sma of the EMA") > 150);
        let back: Vec<f64> = (0..e.len()).map(|i| if i >= 2 { e[i - 2] } else { f64::NAN }).collect();
        assert!(close_enough(&plot(&c, script, 1), &back, "s[2]") > 150);
        // The other way: the built-in SMA over the script's "Double" plot.
        let sma = builtin(&mut c, "sma", &[("length", Value::Int(7)), ("source", link(script, 2))]);
        let double = plot(&c, script, 2);
        assert!(close_enough(&plot(&c, sma, 0), &sma_ref(&double, 7), "SMA of a script plot") > 150);
    }

    /// The use case from PR #7: a signal script in its own pane, read by
    /// another script that acts on it. Live ticks keep them in step.
    #[test]
    fn a_script_reads_another_scripts_signal_live() {
        let mut c = loaded(120);
        let signal = pine(
            &mut c,
            "//@version=6\nindicator(\"Signal\")\nplot(ta.crossover(close, ta.sma(close, 10)) ? 1 : 0, \"Buy\")",
            &[],
        );
        let reader = pine(
            &mut c,
            "//@version=6\nindicator(\"Labels\", overlay = true)\nsig = input.source(close, \"Signal\")\nplot(sig == 1 ? low : na, \"Mark\")\nplot(ta.cum(sig), \"Count\")",
            &[("signal", link(signal, 0))],
        );
        let check = |c: &Chart| {
            let sig = plot(c, signal, 0);
            let mark: Vec<f64> = (0..sig.len())
                .map(|i| if sig[i] == 1.0 { c.bars[i].low } else { f64::NAN })
                .collect();
            close_enough(&plot(c, reader, 0), &mark, "marks on buy bars");
            let mut total = 0.0;
            let count: Vec<f64> = sig
                .iter()
                .map(|v| {
                    total += v;
                    total
                })
                .collect();
            close_enough(&plot(c, reader, 1), &count, "running count");
            assert!(total > 0.0, "some buys in the sample");
        };
        check(&c);
        // A new bar and a tick on it: the reader follows the signal.
        let mut b = *c.bars.last().unwrap();
        b.time += 3_600_000;
        b.close += 30.0;
        b.high = b.close + 1.0;
        b.low = b.open.min(b.close) - 1.0;
        c.apply(Event::Tick { chart: 1, bar: b });
        b.close += 5.0;
        b.high = b.close + 1.0;
        c.apply(Event::Tick { chart: 1, bar: b });
        assert_eq!(plot(&c, reader, 0).len(), c.bars.len());
        check(&c);
    }

    /// Changing the source's inputs reruns its readers, through a chain.
    #[test]
    fn a_change_reaches_every_reader_down_the_chain() {
        let mut c = loaded(200);
        let ema = builtin(&mut c, "ema", &[("length", Value::Int(9))]);
        let rsi = builtin(&mut c, "rsi", &[("length", Value::Int(14)), ("source", link(ema, 0))]);
        let sma = builtin(&mut c, "sma", &[("length", Value::Int(5)), ("source", link(rsi, 0))]);
        let meta = by_id("ema").unwrap();
        let mut inputs = c.studies.iter().find(|s| s.uid == ema).unwrap().inst.inputs.clone();
        inputs.set("length", Value::Int(21));
        c.set_inputs(ema, &inputs);
        let _ = meta;
        let r = plot(&c, rsi, 0);
        close_enough(&r, &rsi_ref(&plot(&c, ema, 0), 14), "RSI after the EMA changed");
        close_enough(&plot(&c, sma, 0), &sma_ref(&r, 5), "SMA of that RSI");
        assert_eq!(c.readers_of(ema).len(), 2);
    }

    /// Links survive a save and load, whatever order the studies come back
    /// in; removing the source puts its readers back on their default.
    #[test]
    fn links_survive_a_restart_and_a_removed_source_lets_go() {
        let mut c = loaded(150);
        let ema = builtin(&mut c, "ema", &[("length", Value::Int(9))]);
        let rsi = builtin(&mut c, "rsi", &[("source", link(ema, 0))]);
        let before = plot(&c, rsi, 0);
        let mut state = c.state();
        // The reader saved first: it waits for its source's id.
        state.studies.reverse();
        let mut back = Chart::from_state(1, state);
        back.apply(Event::Bars {
            chart: 1,
            bars: bars(150, 0),
            complete: true,
        });
        let rsi2 = back.studies.iter().find(|s| s.meta().id == "rsi").unwrap().uid;
        let ema2 = back.studies.iter().find(|s| s.meta().id == "ema").unwrap().uid;
        assert_eq!((ema2, rsi2), (ema, rsi), "ids kept");
        close_enough(&plot(&back, rsi2, 0), &before, "the same RSI after a restart");

        back.remove_study(ema2);
        let s = back.studies.iter().find(|s| s.uid == rsi2).unwrap();
        assert_eq!(s.inst.inputs.text("source"), "close", "back on its default");
        let closes: Vec<f64> = back.bars.iter().map(|b| b.close).collect();
        close_enough(&plot(&back, rsi2, 0), &rsi_ref(&closes, 14), "RSI of the close again");
    }

    /// The Source list never offers a loop, and a loop forced in (a hand-made
    /// layout) doesn't hang: each side just reads nothing.
    #[test]
    fn no_loops_offered_and_a_forced_one_is_harmless() {
        let mut c = loaded(80);
        let a = builtin(&mut c, "ema", &[]);
        let b = builtin(&mut c, "rsi", &[("source", link(a, 0))]);
        let offered = crate::dialogs::link_choices(&c, a);
        assert!(
            !offered
                .iter()
                .any(|(k, _)| opentern_series::study::parse_link(k).unwrap().0 == b)
        );
        assert!(
            !offered
                .iter()
                .any(|(k, _)| opentern_series::study::parse_link(k).unwrap().0 == a)
        );
        assert!(
            crate::dialogs::link_choices(&c, b)
                .iter()
                .any(|(k, _)| opentern_series::study::parse_link(k).unwrap().0 == a)
        );
        let mut inputs = c.studies.iter().find(|s| s.uid == a).unwrap().inst.inputs.clone();
        inputs.set("source", link(b, 0));
        c.set_inputs(a, &inputs);
        c.apply(Event::Bars {
            chart: 1,
            bars: bars(81, 0),
            complete: true,
        });
        assert_eq!(plot(&c, a, 0).len(), 81);
    }

    // ---- more combinations, each checked against a separate run
    /// Every built-in with a Source input, reading an EMA, gives exactly what
    /// it gives when the EMA's values are the bars' close: the link only
    /// changes what "source" means, nothing else.
    #[test]
    fn every_builtin_with_a_source_reads_a_line_exactly() {
        let mut checked = Vec::new();
        let mut wrong = Vec::new();
        for meta in opentern_series::builtins::all() {
            let srcs: Vec<&str> = meta
                .inputs
                .iter()
                .filter(|i| matches!(i.kind, opentern_series::study::InputKind::Source))
                .map(|i| i.id)
                .collect();
            // These add an ATR of the real bars (high, low and close, as on
            // TradingView) to their source, so made-up closes change them:
            // their source-only line is checked instead.
            let atr = ["kc", "median", "volatility_stop"].contains(&meta.id);
            if srcs.is_empty() {
                continue;
            }
            for id in &srcs {
                let mut c = loaded(400);
                let ema = builtin(&mut c, "ema", &[("length", Value::Int(9))]);
                let x = builtin(&mut c, meta.id, &[(id, link(ema, 0))]);
                let e = plot(&c, ema, 0);
                // The same study on bars whose close is the EMA.
                let fake: Vec<Bar> = c.bars.iter().zip(&e).map(|(b, v)| Bar { close: *v, ..*b }).collect();
                let mut inputs = meta.default_inputs();
                inputs.set(id, Value::Text("close".into()));
                let mut alone = Instance::new(meta, &inputs);
                alone.recompute(&fake, &c.sessions, c.tf);
                let got = &c.studies.iter().find(|s| s.uid == x).unwrap().inst.out.plots;
                let mut ok = true;
                for (k, want) in alone.out.plots.iter().enumerate() {
                    // Keltner's and Median's middle lines are the source's alone.
                    if atr && !(k == 1 && meta.id != "volatility_stop") {
                        continue;
                    }
                    for (i, (a, b)) in got[k].iter().zip(want).enumerate() {
                        let same = (a.is_nan() && b.is_nan()) || (a - b).abs() <= 1e-9 * b.abs().max(1.0);
                        if !same {
                            wrong.push(format!("{} {id} plot {k} bar {i}: {a} vs {b}", meta.id));
                            ok = false;
                            break;
                        }
                    }
                }
                if ok {
                    checked.push(format!("{}:{id}", meta.id));
                }
            }
        }
        eprintln!("checked {}: {}", checked.len(), checked.join(" "));
        assert!(wrong.is_empty(), "{}", wrong.join("\n"));
        assert!(checked.len() > 20);
    }

    /// DEMA, as in testing: RSI(14) of DEMA(9) is Wilder's RSI of
    /// 2·EMA − EMA(EMA), worked out here.
    #[test]
    fn rsi_of_a_dema() {
        let mut c = loaded(300);
        let dema = builtin(&mut c, "dema", &[("length", Value::Int(9))]);
        let rsi = builtin(&mut c, "rsi", &[("length", Value::Int(14)), ("source", link(dema, 0))]);
        let closes: Vec<f64> = c.bars.iter().map(|b| b.close).collect();
        let ema = |x: &[f64], n: usize| -> Vec<f64> {
            // Pine's ta.ema: seeded with the SMA of the first n numbers.
            let a = 2.0 / (n as f64 + 1.0);
            let mut out = vec![f64::NAN; x.len()];
            let start = x.iter().position(|v| v.is_finite()).unwrap_or(x.len());
            for i in start..x.len() {
                let p = if i > 0 { out[i - 1] } else { f64::NAN };
                out[i] = if p.is_finite() {
                    a * x[i] + (1.0 - a) * p
                } else if i + 1 >= start + n {
                    x[i + 1 - n..=i].iter().sum::<f64>() / n as f64
                } else {
                    f64::NAN
                };
            }
            out
        };
        let e1 = ema(&closes, 9);
        let e2 = ema(&e1, 9);
        let d: Vec<f64> = e1.iter().zip(&e2).map(|(a, b)| 2.0 * a - b).collect();
        assert!(close_enough(&plot(&c, dema, 0), &d, "DEMA") > 250);
        assert!(close_enough(&plot(&c, rsi, 0), &rsi_ref(&d, 14), "RSI of DEMA") > 250);
    }

    /// A line other than the first (Bollinger's upper band), read by Pine.
    #[test]
    fn a_reader_takes_the_line_it_was_given() {
        let mut c = loaded(250);
        let bb = builtin(&mut c, "bb", &[]);
        let names: Vec<&str> = c
            .studies
            .iter()
            .find(|s| s.uid == bb)
            .unwrap()
            .meta()
            .plots
            .iter()
            .map(|p| p.title)
            .collect();
        let upper = names
            .iter()
            .position(|n| n.to_lowercase().contains("upper"))
            .expect("an upper band");
        let script = pine(
            &mut c,
            "//@version=6\nindicator(\"R\")\ns = input.source(close, \"Src\")\nplot(ta.rsi(s, 14), \"RSI\")\nplot(ta.stdev(s, 10), \"Dev\")",
            &[("src", link(bb, upper))],
        );
        let u = plot(&c, bb, upper);
        assert!(close_enough(&plot(&c, script, 0), &rsi_ref(&u, 14), "ta.rsi of the upper band") > 200);
        let dev: Vec<f64> = (0..u.len())
            .map(|i| {
                if i + 1 < 10 || u[i + 1 - 10..=i].iter().any(|v| !v.is_finite()) {
                    return f64::NAN;
                }
                let w = &u[i + 1 - 10..=i];
                let m = w.iter().sum::<f64>() / 10.0;
                (w.iter().map(|v| (v - m).powi(2)).sum::<f64>() / 10.0).sqrt()
            })
            .collect();
        assert!(close_enough(&plot(&c, script, 1), &dev, "ta.stdev of the upper band") > 200);
    }

    /// A hidden source still feeds its readers.
    #[test]
    fn a_hidden_source_still_feeds() {
        let mut c = loaded(200);
        let ema = builtin(&mut c, "ema", &[("length", Value::Int(9))]);
        let rsi = builtin(&mut c, "rsi", &[("source", link(ema, 0))]);
        c.study_mut(ema).unwrap().visible = false;
        let mut b = *c.bars.last().unwrap();
        b.time += 3_600_000;
        c.apply(Event::Tick { chart: 1, bar: b });
        let e = plot(&c, ema, 0);
        assert_eq!(e.len(), c.bars.len());
        close_enough(&plot(&c, rsi, 0), &rsi_ref(&e, 14), "RSI of a hidden EMA");
    }

    /// Luau and JavaScript scripts as sources: a built-in and a Pine script
    /// read their plots.
    #[test]
    fn luau_and_js_scripts_feed_others() {
        let langs: [(std::sync::Arc<dyn opentern_script::inproc::Lang>, &str, &str); 2] = [
            (
                std::sync::Arc::new(opentern_script::luau::Luau),
                "t.luau",
                "indicator(\"L\")\nfunction on_bar(bar)\n  plot((bar.high + bar.low) / 2 + bar.close * 0.01, \"Mid\")\nend\n",
            ),
            (
                std::sync::Arc::new(opentern_script::js::Js { ts: false }),
                "t.js",
                "indicator(\"J\");\nfunction on_bar(bar) {\n  plot((bar.high + bar.low) / 2 + bar.close * 0.01, \"Mid\");\n}\n",
            ),
        ];
        for (lang, file, src) in langs {
            let mut c = loaded(200);
            let sym = c.symbol.clone();
            let comp = opentern_script::inproc::compile(lang, src, file, crate::pine_host::syminfo_of(&sym))
                .unwrap_or_else(|e| panic!("{file}: {e}"));
            let inst = Instance::with_factory(comp.meta(), &comp.meta().default_inputs(), comp.factory());
            let s = c.add_instance(inst, StudyLook::defaults(comp.meta()), None, true);
            let want: Vec<f64> = c.bars.iter().map(|b| (b.high + b.low) / 2.0 + b.close * 0.01).collect();
            close_enough(&plot(&c, s, 0), &want, file);
            let sma = builtin(&mut c, "sma", &[("length", Value::Int(6)), ("source", link(s, 0))]);
            close_enough(&plot(&c, sma, 0), &sma_ref(&want, 6), "SMA of the script");
            let p = pine(
                &mut c,
                "//@version=6\nindicator(\"P\")\ns = input.source(close, \"Src\")\nplot(ta.rsi(s, 14))",
                &[("src", link(s, 0))],
            );
            close_enough(&plot(&c, p, 0), &rsi_ref(&want, 14), "Pine RSI of the script");
        }
    }

    /// Luau and JavaScript scripts read another indicator's line: their
    /// `input.source` returns a name, and `bar[name]` is the line's value.
    #[test]
    fn luau_and_js_scripts_read_a_line() {
        let langs: [(std::sync::Arc<dyn opentern_script::inproc::Lang>, &str, &str); 2] = [
            (
                std::sync::Arc::new(opentern_script::luau::Luau),
                "r.luau",
                "indicator(\"L\")\nlocal src = input.source(\"Src\", \"close\")\nfunction on_bar(bar)\n  plot(ta.sma(bar[src], 5), \"Avg\")\n  plot(bar[src], \"Raw\")\nend\n",
            ),
            (
                std::sync::Arc::new(opentern_script::js::Js { ts: false }),
                "r.js",
                "indicator(\"J\");\nconst src = input.source(\"Src\", \"close\");\nfunction on_bar(bar) {\n  plot(ta.sma(bar[src], 5), \"Avg\");\n  plot(bar[src], \"Raw\");\n}\n",
            ),
        ];
        for (lang, file, src) in langs {
            let mut c = loaded(200);
            let ema = builtin(&mut c, "ema", &[("length", Value::Int(9))]);
            let sym = c.symbol.clone();
            let comp = opentern_script::inproc::compile(lang, src, file, crate::pine_host::syminfo_of(&sym))
                .unwrap_or_else(|e| panic!("{file}: {e}"));
            let mut inputs = comp.meta().default_inputs();
            inputs.set("src", link(ema, 0));
            let inst = Instance::with_factory(comp.meta(), &inputs, comp.factory());
            let s = c.add_instance(inst, StudyLook::defaults(comp.meta()), None, true);
            let e = plot(&c, ema, 0);
            assert!(close_enough(&plot(&c, s, 1), &e, file) > 150, "{file}: the line itself");
            assert!(
                close_enough(&plot(&c, s, 0), &sma_ref(&e, 5), file) > 150,
                "{file}: ta.sma of it"
            );
            // A change to the EMA reaches the script.
            let mut ei = c.studies.iter().find(|x| x.uid == ema).unwrap().inst.inputs.clone();
            ei.set("length", Value::Int(20));
            c.set_inputs(ema, &ei);
            close_enough(&plot(&c, s, 1), &plot(&c, ema, 0), file);
            // And the Source list offers it.
            assert!(!crate::dialogs::link_choices(&c, s).is_empty(), "{file}: offered");
        }
    }

    /// A Pine strategy trading on another indicator's line: it enters and
    /// leaves on the bars the RSI it reads crosses 50, worked out here.
    #[test]
    fn a_strategy_trades_on_a_linked_line() {
        let mut c = loaded(400);
        let ema = builtin(&mut c, "ema", &[("length", Value::Int(5))]);
        let rsi = builtin(&mut c, "rsi", &[("length", Value::Int(10)), ("source", link(ema, 0))]);
        let st = pine(
            &mut c,
            "//@version=6\nstrategy(\"S\")\ns = input.source(close, \"Signal\")\nif ta.crossover(s, 50)\n    strategy.entry(\"L\", strategy.long)\nif ta.crossunder(s, 50)\n    strategy.close(\"L\")\nplot(s)",
            &[("signal", link(rsi, 0))],
        );
        let r = plot(&c, rsi, 0);
        let up: Vec<usize> = (1..r.len()).filter(|&i| r[i - 1] <= 50.0 && r[i] > 50.0).collect();
        let down: Vec<usize> = (1..r.len()).filter(|&i| r[i - 1] >= 50.0 && r[i] < 50.0).collect();
        let s = c.studies.iter().find(|s| s.uid == st).unwrap();
        let p = s
            .inst
            .state()
            .and_then(|a| a.downcast_ref::<opentern_pine::PineStudy>())
            .unwrap();
        assert!(p.error().is_none(), "{:?}", p.error());
        let trades = &p.broker().unwrap().trades;
        assert!(trades.len() >= 3, "{} trades", trades.len());
        for t in trades {
            // Market orders fill on the next bar's open.
            assert!(
                up.contains(&(t.entry_bar - 1)),
                "entry at {} not after a cross up",
                t.entry_bar
            );
            assert!(
                down.contains(&(t.exit_bar - 1)),
                "exit at {} not after a cross down",
                t.exit_bar
            );
            assert_eq!(t.entry_price, c.bars[t.entry_bar].open);
        }
    }

    /// Pine → built-in → Pine, three deep, and every step's numbers right.
    #[test]
    fn a_mixed_chain_three_deep() {
        let mut c = loaded(300);
        let a = pine(
            &mut c,
            "//@version=6\nindicator(\"A\")\nplot(close - open, \"Body\")",
            &[],
        );
        let b = builtin(&mut c, "sma", &[("length", Value::Int(4)), ("source", link(a, 0))]);
        let z = pine(
            &mut c,
            "//@version=6\nindicator(\"Z\")\ns = input.source(close, \"Src\")\nvar float hi = na\nhi := na(s) ? hi : math.max(nz(hi, s), s)\nplot(hi, \"Max so far\")",
            &[("src", link(b, 0))],
        );
        let body: Vec<f64> = c.bars.iter().map(|b| b.close - b.open).collect();
        let avg = sma_ref(&body, 4);
        close_enough(&plot(&c, b, 0), &avg, "SMA of the body");
        let mut hi = f64::NAN;
        let want: Vec<f64> = avg
            .iter()
            .map(|v| {
                if v.is_finite() {
                    hi = if hi.is_finite() { hi.max(*v) } else { *v };
                }
                hi
            })
            .collect();
        close_enough(&plot(&c, z, 0), &want, "running max of it");
    }

    #[test]
    fn a_live_chart_whose_prices_stop_moving_says_so() {
        let mut c = chart();
        assert!(c.symbol.id.starts_with("BINANCE:"), "crypto: always open");
        let now = jiff::Timestamp::now().as_millisecond();
        let b = bars(3, 0);
        c.apply(Event::Bars {
            chart: 1,
            bars: b.clone(),
            complete: false,
        });
        c.apply(Event::Status {
            chart: 1,
            status: Status::Live,
        });
        assert_eq!(c.stale_for(now), None, "just arrived");
        let long_ago = std::time::Instant::now() - STALE_AFTER - std::time::Duration::from_secs(60);
        c.moved = Some(long_ago);
        assert!(c.stale_for(now).is_some_and(|d| d >= STALE_AFTER));
        // The same bar again is no news.
        c.apply(Event::Tick { chart: 1, bar: b[2] });
        assert!(c.stale_for(now).is_some());
        // A changed price is.
        c.apply(Event::Tick {
            chart: 1,
            bar: Bar {
                close: b[2].close + 1.0,
                ..b[2]
            },
        });
        assert_eq!(c.stale_for(now), None);
        // Offline says so itself.
        c.moved = Some(long_ago);
        c.apply(Event::Status {
            chart: 1,
            status: Status::Offline("x".into()),
        });
        assert_eq!(c.stale_for(now), None);
    }

    #[test]
    fn known_gaps_in_yahoos_indian_prices_are_said() {
        use opentern_core::{AssetKind, Symbol};
        let ms = |s: &str| s.parse::<jiff::Timestamp>().unwrap().as_millisecond();
        let day = |d: &str| ms(&format!("{d}T00:00:00Z"));
        let bar = |t: i64| Bar {
            time: t,
            open: 1.0,
            high: 1.0,
            low: 1.0,
            close: 1.0,
            volume: 1.0,
        };
        let open = |id: &str, tf: &str, bars: Vec<Bar>| {
            let sym = Symbol {
                kind: AssetKind::Stock,
                timezone: "Asia/Kolkata".into(),
                ..Symbol::bare(id)
            };
            let mut c = Chart::new(1, sym, Timeframe::parse(tf).unwrap());
            c.apply(Event::Bars {
                chart: 1,
                bars,
                complete: true,
            });
            c.apply(Event::Status {
                chart: 1,
                status: Status::Live,
            });
            c
        };
        let now = ms("2026-11-12T06:00:00Z");
        // BSE daily history that's one bar.
        let c = open("BSE:INFY", "1D", vec![bar(day("2026-11-12"))]);
        assert!(c.data_gap(now).unwrap().contains("NSE:INFY"));
        // Muhurat trading on Sunday 8 Nov 2026, missing between Friday and Monday.
        let days = vec![bar(day("2026-11-06")), bar(day("2026-11-09")), bar(day("2026-11-10"))];
        let c = open("NSE:INFY", "1D", days.clone());
        assert!(c.data_gap(now).unwrap().contains("8 Nov 2026"), "{:?}", c.data_gap(now));
        let mut with = days.clone();
        with.insert(1, bar(day("2026-11-08")));
        assert_eq!(open("NSE:INFY", "1D", with).data_gap(now), None, "built from the quote");
        // Over a month later, or a chart that starts after it: nothing to say.
        assert_eq!(c.data_gap(ms("2026-12-20T06:00:00Z")), None);
        assert_eq!(open("NSE:INFY", "1D", days[1..].to_vec()).data_gap(now), None);
        // Intraday: no bar within the session's window.
        let c = open(
            "NSE:INFY",
            "15m",
            vec![bar(ms("2026-11-06T09:00:00Z")), bar(ms("2026-11-09T04:00:00Z"))],
        );
        assert!(c.data_gap(now).is_some());
        let c = open(
            "NSE:INFY",
            "15m",
            vec![bar(ms("2026-11-06T09:00:00Z")), bar(ms("2026-11-08T09:00:00Z"))],
        );
        assert_eq!(c.data_gap(now), None);
        // US stocks and weekly charts: never.
        assert_eq!(open("NSE:INFY", "1W", days).data_gap(now), None);
    }

    #[test]
    fn a_closed_market_is_never_stale() {
        let sym = opentern_core::Symbol::bare("NASDAQ:AAPL");
        let mut c = Chart::new(1, sym.clone(), Timeframe::parse("1h").unwrap());
        c.apply(Event::Bars {
            chart: 1,
            bars: bars(3, 0),
            complete: false,
        });
        c.apply(Event::Status {
            chart: 1,
            status: Status::Live,
        });
        c.moved = Some(std::time::Instant::now() - STALE_AFTER * 3);
        // Saturday 14 Nov 2026, noon in New York; and Monday 16 Nov at 09:32,
        // two minutes after the open (the night's quiet doesn't count).
        let sat = "2026-11-14T17:00:00Z"
            .parse::<jiff::Timestamp>()
            .unwrap()
            .as_millisecond();
        let open = "2026-11-16T14:32:00Z"
            .parse::<jiff::Timestamp>()
            .unwrap()
            .as_millisecond();
        let later = "2026-11-16T15:30:00Z"
            .parse::<jiff::Timestamp>()
            .unwrap()
            .as_millisecond();
        assert_eq!(c.stale_for(sat), None);
        assert_eq!(c.stale_for(open), None);
        assert!(c.stale_for(later).is_some());
        assert!(!steadily_open(&sym, sat));
    }

    fn add(c: &mut Chart, id: &str) -> u64 {
        let m = by_id(id).unwrap();
        c.add_study(m, &m.default_inputs(), StudyLook::defaults(m), None)
    }

    #[test]
    fn pane_studies_get_their_own_pane_and_it_goes_with_them() {
        let mut c = chart();
        add(&mut c, "sma");
        let rsi = add(&mut c, "rsi");
        add(&mut c, "macd");
        assert_eq!(c.panes.len(), 3);
        c.remove_study(rsi);
        assert_eq!(c.panes.len(), 2);
        assert!(
            c.studies
                .iter()
                .all(|s| s.pane == 0 || c.panes.iter().any(|p| p.id == s.pane))
        );
    }

    #[test]
    fn state_round_trips_through_json() {
        let mut c = chart();
        add(&mut c, "bb");
        let rsi = add(&mut c, "rsi");
        c.study_mut(rsi).unwrap().look.plots[0].width = 3;
        c.style.up_body = opentern_series::Rgba::hex(0x123456);
        let json = serde_json::to_string(&c.state()).unwrap();
        let back = Chart::from_state(2, serde_json::from_str(&json).unwrap());
        assert_eq!(back.studies.len(), 2);
        assert_eq!(back.panes.len(), 2);
        assert_eq!(back.style.up_body, c.style.up_body);
        let r = back.studies.iter().find(|s| s.meta().id == "rsi").unwrap();
        assert_eq!(r.look.plots[0].width, 3);
        // A layout naming a study that doesn't exist still loads.
        let json = json.replace("\"rsi\"", "\"no-such-study\"");
        let back = Chart::from_state(3, serde_json::from_str(&json).unwrap());
        assert_eq!(back.studies.len(), 1);
        assert_eq!(back.panes.len(), 1);
    }

    #[test]
    fn live_ticks_match_a_full_recompute() {
        let all = bars(400, 0);
        let mut live = chart();
        add(&mut live, "rsi");
        add(&mut live, "macd");
        live.apply(Event::Bars {
            chart: 1,
            bars: all[..300].to_vec(),
            complete: true,
        });
        for b in &all[300..] {
            let mut forming = *b;
            forming.close = forming.open;
            live.apply(Event::Tick { chart: 1, bar: forming });
            live.apply(Event::Tick { chart: 1, bar: *b });
        }
        let mut full = chart();
        add(&mut full, "rsi");
        add(&mut full, "macd");
        full.apply(Event::Bars {
            chart: 1,
            bars: all.clone(),
            complete: true,
        });
        assert_eq!(live.bars, full.bars);
        for (a, b) in live.studies.iter().zip(&full.studies) {
            for (pa, pb) in a.inst.out.plots.iter().zip(&b.inst.out.plots) {
                for (x, y) in pa.iter().zip(pb) {
                    assert!((x.is_nan() && y.is_nan()) || (x - y).abs() < 1e-9);
                }
            }
        }
    }

    #[test]
    fn older_history_keeps_the_view_in_place() {
        let mut c = chart();
        c.apply(Event::Bars {
            chart: 1,
            bars: bars(100, 100),
            complete: false,
        });
        // Scroll back, away from the latest bar.
        c.view.right = 40.0;
        let shown_time = c.bars[40].time;
        c.apply(Event::Bars {
            chart: 1,
            bars: bars(200, 0),
            complete: true,
        });
        assert_eq!(c.bars[c.view.right as usize].time, shown_time);
        assert!(c.complete);
    }

    #[test]
    fn following_view_moves_with_new_bars() {
        let mut c = chart();
        c.apply(Event::Bars {
            chart: 1,
            bars: bars(50, 0),
            complete: true,
        });
        let r = c.view.right;
        c.apply(Event::Tick {
            chart: 1,
            bar: bars(1, 50)[0],
        });
        assert_eq!(c.view.right, r + 1.0);
    }

    #[test]
    fn indicator_template_replaces_studies() {
        let mut c = chart();
        add(&mut c, "volume");
        add(&mut c, "rsi");
        let tpl = StudyTemplate {
            version: 1,
            studies: c.saved_studies(),
            panes: c.panes.clone(),
        };
        let mut other = chart();
        add(&mut other, "macd");
        add(&mut other, "stoch");
        other.apply_study_template(&tpl);
        let ids: Vec<_> = other.studies.iter().map(|s| s.meta().id).collect();
        assert_eq!(ids, ["volume", "rsi"]);
        assert_eq!(other.panes.len(), 2);
    }
}

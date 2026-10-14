//! A chart: one symbol and interval, its bars, the studies on it, its
//! panes and how it's scrolled and zoomed. Drawing and input live in
//! `paint.rs` and `ui.rs`; this file owns the data and its life cycle.

pub mod draw;
pub mod drawing_ui;
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

/// Another symbol drawn on the chart: as a percent comparison (the price
/// scale switches to percent, as on TradingView's "Compare"), or overlaid
/// with its own scale.
pub struct Compare {
    pub symbol: Symbol,
    pub session: u64,
    pub bars: Vec<Bar>,
    pub status: Status,
    pub color: opentern_series::Rgba,
    pub overlay: bool,
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
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SavedCompare {
    pub symbol: Symbol,
    pub color: opentern_series::Rgba,
    #[serde(default)]
    pub overlay: bool,
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
    /// Dividends and splits (stocks), and whether prices are adjusted for
    /// dividends (TradingView's ADJ). `raw` keeps the unadjusted bars.
    pub corporate: Option<opentern_data::yahoo::Corporate>,
    pub adj: bool,
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
            corporate: None,
            adj: false,
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
        c.percent = s.percent;
        c.compares = s
            .compares
            .into_iter()
            .map(|x| Compare {
                symbol: x.symbol,
                session: 0,
                bars: Vec::new(),
                status: Status::Loading,
                color: x.color,
                overlay: x.overlay,
                visible: x.visible,
            })
            .collect();
        if !s.panes.is_empty() && s.panes[0].id == 0 {
            c.panes = s.panes;
        }
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
            .collect();
        c.panes.retain(|p| p.id == 0 || used.contains(&p.id));
        c.next_uid = c
            .studies
            .iter()
            .map(|s| s.uid)
            .chain(c.panes.iter().map(|p| p.id))
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
            percent: self.percent,
            compares: self
                .compares
                .iter()
                .map(|c| SavedCompare {
                    symbol: c.symbol.clone(),
                    color: c.color,
                    overlay: c.overlay,
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
    pub fn add_compare(&mut self, symbol: Symbol, overlay: bool) -> usize {
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
            overlay,
            visible: true,
        });
        if !overlay {
            self.percent = true;
            self.log = false;
        }
        self.touch();
        self.compares.len() - 1
    }

    /// Removes a compared symbol; returns its data session to close.
    pub fn remove_compare(&mut self, i: usize) -> Option<u64> {
        if i >= self.compares.len() {
            return None;
        }
        let c = self.compares.remove(i);
        if !c.overlay && !self.compares.iter().any(|c| !c.overlay) {
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
            })
            .collect()
    }

    pub fn apply_timezone(&mut self) {
        use style::TimeZoneChoice::*;
        self.tz = match self.style.timezone {
            Exchange => self.sym_tz.clone(),
            Utc => TimeZone::UTC,
            Local => TimeZone::system(),
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
        let uid = self.new_uid();
        let mut inst = Instance::new(meta, inputs);
        inst.recompute(&self.bars, &self.sessions, self.tf);
        self.studies.push(StudyView {
            uid,
            inst,
            look: look.fitted(meta),
            visible: true,
            pane,
        });
        self.touch();
        uid
    }

    /// Adds a study whose instance is already built (scripts), in `pane`
    /// (or a new pane for pane studies).
    pub fn add_instance(&mut self, inst: Instance, look: StudyLook, pane: Option<u64>, visible: bool) -> u64 {
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
        let uid = self.new_uid();
        let mut inst = inst;
        inst.recompute(&self.bars, &self.sessions, self.tf);
        self.studies.push(StudyView {
            uid,
            look: look.fitted(meta),
            inst,
            visible,
            pane,
        });
        self.touch();
        uid
    }

    /// Swaps a study's instance (a script whose outputs changed shape),
    /// keeping its place and as much of its look as still fits.
    pub fn replace_instance(&mut self, uid: u64, inst: Instance) {
        let (bars, sess, tf) = (&self.bars, &self.sessions, self.tf);
        if let Some(s) = self.studies.iter_mut().find(|s| s.uid == uid) {
            let mut inst = inst;
            inst.recompute(bars, sess, tf);
            s.look = s.look.fitted(inst.meta);
            s.inst = inst;
        }
    }

    pub fn bars_arc(&self) -> std::sync::Arc<Vec<Bar>> {
        std::sync::Arc::new(self.bars.clone())
    }

    fn add_saved(&mut self, s: &SavedStudy) {
        if s.id.starts_with("py:") || s.id.starts_with("pine:") {
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
        let uid = self.add_study(meta, &s.inputs, s.look.clone(), pane);
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
        let used: Vec<u64> = self.studies.iter().map(|s| s.pane).collect();
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
        self.touch();
    }

    pub fn study_mut(&mut self, uid: u64) -> Option<&mut StudyView> {
        self.studies.iter_mut().find(|s| s.uid == uid)
    }

    /// New inputs for a study: it recomputes from the first bar.
    pub fn set_inputs(&mut self, uid: u64, inputs: &Inputs) {
        let (bars, sess, tf) = (&self.bars, &self.sessions, self.tf);
        if let Some(s) = self.studies.iter_mut().find(|s| s.uid == uid) {
            // Keeps how the study is built (a script's factory, or outputs
            // that come from a Python process).
            s.inst = s.inst.with_inputs(inputs);
            s.inst.recompute(bars, sess, tf);
        }
        self.touch();
    }

    fn recompute_all(&mut self) {
        for s in &mut self.studies {
            s.inst.recompute(&self.bars, &self.sessions, self.tf);
        }
    }

    pub fn last_index(&self) -> f64 {
        self.bars.len() as f64 - 1.0
    }

    /// Whether the view shows the latest bar (and should keep following it).
    pub fn following(&self) -> bool {
        self.view.right >= self.last_index()
    }

    /// Applies a data event meant for this chart.
    pub fn apply(&mut self, e: Event) {
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
        for s in &mut self.studies {
            s.inst.update(&self.bars, &self.sessions, self.tf);
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
    pub fn fit_range(&mut self, width: f32) {
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
        self.fit_days = None;
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

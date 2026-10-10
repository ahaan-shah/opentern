//! The app: owns the workspace, theme, updater, data hub, the chart and the
//! window arrangement, and runs each frame behind a crash shield.

use crate::chart::draw::{DrawStyle, Drawing, Tool};
use crate::chart::style::ChartStyle;
use crate::chart::ui::{Action, ChartUi};
use crate::chart::{Chart, StudyDefault, StudyLook, StudyTemplate};
use crate::dialogs::{Modal, TemplateKind};
use crate::shell::{Arrangement, Tab};
use crate::theme::{self, Theme, ThemeState};
use crate::updater::{Phase, Updater};
use opentern_core::{Settings, Store, Symbol, Timeframe};
use opentern_data::{Event, Hub, Request};
use opentern_series::Meta;
use opentern_series::study::Inputs;
use std::collections::HashMap;
use std::time::{Duration, Instant};

pub struct App {
    pub store: Store,
    pub theme: ThemeState,
    pub updater: Updater,
    pub layout: Arrangement,
    /// The tab the bottom panel reopens on.
    pub last_tab: Tab,
    pub watchlists: Vec<crate::watchlists::Watchlist>,
    pub active_list: usize,
    /// Last price and previous close per symbol id.
    pub quotes: HashMap<String, (f64, f64)>,
    pub hub: Hub,
    pub chart: Chart,
    pub chart_ui: ChartUi,
    pub modal: Option<Modal>,
    /// The named layout being worked on (autosaved).
    pub layout_id: i64,
    pub layout_name: String,
    pub study_menu: Option<(u64, egui::Pos2)>,
    pub scripts: Vec<crate::scripting::ScriptRun>,
    /// Strategy runs and the Strategy Report's state.
    pub strategies: crate::strategy::Strategies,
    /// The paper trading account and its ticket.
    pub paper: crate::paper::Paper,
    /// Alerts, their log and toasts.
    pub alerts: crate::alerts::Alerts,
    pub editor: crate::editor::Editor,
    pub screener: crate::screener::Screener,
    pub notes: crate::notes::Notes,
    pub side: crate::side::Side,
    /// A snapshot waiting for the window's screenshot.
    pub(crate) snap: Option<crate::snapshot::Snap>,
    /// Where the charts were drawn last frame (snapshots cut this out).
    pub(crate) grid_rect: egui::Rect,
    pub egui_ctx: egui::Context,
    /// Per-tool default drawing styles ("Save as default" on the drawing bar).
    pub(crate) draw_styles: HashMap<Tool, DrawStyle>,
    /// The other charts of a multi-chart layout (`None` at the active
    /// chart's place; see `multi.rs`).
    pub slots: Vec<Option<crate::multi::Slot>>,
    pub active: usize,
    pub multi: crate::multi::Multi,
    pub(crate) drawings_saved: u64,
    last_style_edit: Option<(u64, Instant)>,
    search_seq: u64,
    saved: u64,
    last_save: Instant,
    last_frame: Option<Instant>,
    pub(crate) panics: u32,
}

/// Maximum frames per second while something is animating.
const FRAME_CAP: f32 = 144.0;

/// Intervals shown in the top bar until someone stars their own.
pub const DEFAULT_INTERVALS: [&str; 6] = ["1m", "5m", "15m", "1h", "4h", "1D"];

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, store: Store, demo: bool) -> App {
        let layout: Arrangement = cc
            .storage
            .and_then(|s| eframe::get_value(s, "layout"))
            .unwrap_or_default();
        App::with_context(&cc.egui_ctx, layout, store, demo)
    }

    /// Builds the app against any egui context (the real window, or a
    /// headless one in tests).
    pub(crate) fn with_context(ctx: &egui::Context, layout: Arrangement, store: Store, demo: bool) -> App {
        theme::install_fonts(ctx, &store.settings().font);
        let theme = ThemeState::new(ctx, &store.settings().theme);
        let wake_ctx = ctx.clone();
        let hub = Hub::start(
            store.dir().join("cache"),
            demo,
            std::sync::Arc::new(move || wake_ctx.request_repaint()),
        );
        // The layout last worked on, else the most recent one.
        let current = store
            .value("layout_id")
            .and_then(|v| v.parse::<i64>().ok())
            .and_then(|id| store.layouts().iter().find(|l| l.id == id).cloned())
            .or_else(|| store.layouts().first().cloned());
        let (layout_id, layout_name) = current
            .as_ref()
            .map(|l| (l.id, l.name.clone()))
            .unwrap_or((1, "Untitled".to_string()));
        let saved = current
            .and_then(|l| store.layout_json(l.id).ok().flatten())
            .and_then(|j| crate::multi::LayoutState::parse(&j));
        let fresh = saved.is_none();
        let chart = {
            // A brand-new workspace: Bitcoin hourly with volume, in the
            // default chart template if one was made.
            let btc = opentern_core::starter_watchlist().remove(0);
            let mut c = Chart::new(1, btc, Timeframe::parse("1h").expect("valid"));
            if let Some(t) = store.default_template("chart")
                && let Ok(s) = serde_json::from_str::<ChartStyle>(&t.json)
            {
                c.style = s;
                c.apply_timezone();
            }
            if let Some(v) = opentern_series::builtins::by_id("volume") {
                c.add_study(v, &v.default_inputs(), StudyLook::defaults(v), None);
            }
            c
        };
        let (watchlists, active_list) = App::load_watchlists(&store);
        let paper = crate::paper::Paper::load(&store);
        let alerts = crate::alerts::Alerts::load(&store);
        let mut app = App {
            store,
            theme,
            updater: Updater::default(),
            last_tab: layout.tab.unwrap_or(Tab::Editor),
            layout,
            watchlists,
            active_list,
            quotes: HashMap::new(),
            hub,
            saved: u64::MAX,
            chart,
            chart_ui: ChartUi::default(),
            modal: None,
            layout_id,
            layout_name,
            study_menu: None,
            scripts: Vec::new(),
            strategies: Default::default(),
            paper,
            alerts,
            editor: Default::default(),
            screener: Default::default(),
            notes: Default::default(),
            side: Default::default(),
            snap: None,
            grid_rect: egui::Rect::NOTHING,
            egui_ctx: ctx.clone(),
            draw_styles: HashMap::new(),
            slots: vec![None],
            active: 0,
            multi: Default::default(),
            drawings_saved: 0,
            last_style_edit: None,
            search_seq: 0,
            last_save: Instant::now(),
            last_frame: None,
            panics: 0,
        };
        for t in app.store.templates("drawing") {
            if let (Some(tool), Ok(style)) = (
                crate::chart::draw::Group::ALL
                    .iter()
                    .flat_map(|g| g.tools())
                    .find(|x| x.key() == t.name),
                serde_json::from_str::<DrawStyle>(&t.json),
            ) {
                app.draw_styles.insert(*tool, style);
            }
        }
        app.send_credentials();
        match saved {
            Some(l) => app.install_layout(l),
            None => {
                app.load_drawings();
                app.open_chart();
            }
        }
        if fresh {
            app.saved = app.layout_version();
        }
        app.subscribe_quotes();
        app.still();
        app
    }

    /// For screenshots: `OPENTERN_STILL=indicators|symbol|chart-settings|
    /// study:<id>|save-template` opens that dialog at launch;
    /// `script:<file>` puts a workspace script on the chart, `add:<id>` a
    /// built-in.
    fn still(&mut self) {
        let Ok(what) = std::env::var("OPENTERN_STILL") else {
            return;
        };
        // Several at once: "script:x.pine,tab:report".
        let mut rest = Vec::new();
        for part in what.split(',') {
            if let Some(file) = part.strip_prefix("script:") {
                self.add_script(file);
            } else if let Some(id) = part.strip_prefix("add:") {
                if let Some(m) = opentern_series::builtins::by_id(id) {
                    self.add_study(m);
                }
            } else if let Some(g) = part.strip_prefix("grid:") {
                if let Some(g) = crate::multi::Grid::ALL
                    .into_iter()
                    .find(|x| format!("{x:?}").eq_ignore_ascii_case(g))
                {
                    self.set_grid(g);
                }
            } else if let Some(sym) = part.strip_prefix("compare:") {
                let found = opentern_core::starter_watchlist()
                    .into_iter()
                    .find(|s| s.ticker().eq_ignore_ascii_case(sym));
                if let Some(s) = found {
                    self.add_compare(s, false);
                }
            } else if let Some(tf) = part.strip_prefix("tf:").and_then(Timeframe::parse) {
                self.set_interval(tf);
            } else if let Some(w) = part.strip_prefix("widget:") {
                self.layout.widget = crate::shell::Widget::ALL
                    .into_iter()
                    .find(|x| x.title().to_lowercase().starts_with(&w.to_lowercase()));
            } else if part == "screen" {
                self.layout.tab = Some(crate::shell::Tab::Screener);
                self.run_screener();
            } else if part == "replay" {
                self.chart.toggle_replay();
            } else if let Some(kind) = part.strip_prefix("kind:") {
                let want = kind.to_lowercase().replace(['-', '_', ' '], "");
                if let Some(k) = crate::chart::style::SeriesKind::ALL
                    .into_iter()
                    .find(|k| k.name().to_lowercase().replace(['-', ' ', '&'], "") == want)
                {
                    let mut s = self.chart.style.clone();
                    s.kind = k;
                    self.set_chart_style(s);
                }
            } else if let Some(tab) = part.strip_prefix("tab:") {
                self.layout.tab = crate::shell::Tab::ALL
                    .into_iter()
                    .find(|t| t.title().to_lowercase().starts_with(&tab.to_lowercase()));
            } else {
                rest.push(part);
            }
        }
        let what = rest.join(",");
        if what.is_empty() {
            return;
        }
        self.modal = match what.as_str() {
            "indicators" => Some(Modal::Indicators(crate::dialogs::Indicators::new(false))),
            "symbol" => Some(Modal::Symbol(crate::dialogs::SymbolSearch::new(""))),
            "chart-settings" => Some(Modal::ChartSettings(crate::dialogs::ChartSettings::new(self))),
            "providers" => {
                self.open_providers();
                return;
            }
            "alert" => {
                self.open_alert_dialog(None);
                return;
            }
            "save-template" => Some(Modal::SaveTemplate {
                kind: TemplateKind::Indicators,
                name: String::new(),
            }),
            s => s.strip_prefix("study:").and_then(|id| {
                let uid = self.chart.studies.iter().find(|x| x.meta().id == id)?.uid;
                crate::dialogs::StudySettings::new(self, uid).map(Modal::Study)
            }),
        };
    }

    pub fn t(&self) -> Theme {
        self.theme.current
    }

    /// (Re)opens the chart's data. Each opening gets a new session id, so
    /// late events from the previous symbol or interval are ignored.
    fn open_chart(&mut self) {
        self.hub.send(Request::Close { chart: self.chart.id });
        self.chart.id = crate::chart::new_session_id();
        self.hub.send(Request::Open {
            chart: self.chart.id,
            symbol: self.chart.symbol.clone(),
            tf: self.chart.tf,
        });
        for i in 0..self.chart.compares.len() {
            self.open_compare(i);
        }
    }

    pub fn toggle_dark(&mut self, ctx: &egui::Context) {
        let name = if self.t().dark {
            "Opentern Light"
        } else {
            "Opentern Dark"
        };
        self.theme.switch(ctx, name);
        let s = Settings {
            theme: name.into(),
            ..self.store.settings().clone()
        };
        if let Err(e) = self.store.set_settings(s) {
            crate::diag::crumb(format!("couldn't save theme: {e}"));
        }
    }

    pub fn update_action(&mut self, ctx: &egui::Context) {
        match &self.updater.phase {
            Phase::Available(_) => self.updater.install(ctx),
            Phase::Manual(r, _) => open_url(&r.page),
            Phase::Ready(_) => self.updater.restart(ctx),
            _ => {}
        }
    }

    // ------------------------------------------------------------ chart

    pub fn search(&mut self, seq: u64, query: &str) {
        self.search_seq = seq;
        self.hub.send(Request::Search {
            seq,
            query: query.to_string(),
        });
    }

    pub fn open_symbol(&mut self, symbol: Symbol) {
        crate::diag::crumb(format!("symbol {}", symbol.id));
        self.save_drawings(true);
        let tf = self.chart.tf;
        self.chart.reset_series(symbol, tf);
        self.chart_ui.draw = Default::default();
        self.load_drawings();
        self.open_chart();
    }

    // --------------------------------------------------------- drawings

    pub fn save_drawing_default(&mut self, tool: Tool, style: &DrawStyle) {
        if let Ok(j) = serde_json::to_string(style)
            && self.store.save_template("drawing", &tool.key(), &j).is_ok()
        {
            self.draw_styles.insert(tool, style.clone());
        }
    }

    pub(crate) fn reload_drawings(&mut self) {
        self.load_drawings();
    }

    pub(crate) fn save_drawings_now(&mut self) {
        self.save_drawings(true);
    }

    pub(crate) fn open_chart_data(&mut self) {
        self.open_chart();
    }

    /// (Re)opens a compared symbol's data on the chart's interval.
    pub(crate) fn open_compare(&mut self, i: usize) {
        let tf = self.chart.tf;
        let Some(c) = self.chart.compares.get_mut(i) else {
            return;
        };
        if c.session != 0 {
            self.hub.send(Request::Close { chart: c.session });
        }
        c.session = crate::chart::new_session_id();
        c.bars.clear();
        self.hub.send(Request::Open {
            chart: c.session,
            symbol: c.symbol.clone(),
            tf,
        });
    }

    pub fn add_compare(&mut self, symbol: Symbol, overlay: bool) {
        if symbol.id == self.chart.symbol.id || self.chart.compares.iter().any(|c| c.symbol.id == symbol.id) {
            return;
        }
        crate::diag::crumb(format!("compare {}", symbol.id));
        let i = self.chart.add_compare(symbol, overlay);
        self.open_compare(i);
    }

    pub fn remove_compare(&mut self, i: usize) {
        if let Some(session) = self.chart.remove_compare(i) {
            self.hub.send(Request::Close { chart: session });
        }
    }

    /// Stops the chart's data (it's leaving the layout).
    pub(crate) fn close_chart_data(&mut self) {
        self.hub.send(Request::Close { chart: self.chart.id });
        for c in &self.chart.compares {
            self.hub.send(Request::Close { chart: c.session });
        }
    }

    fn load_drawings(&mut self) {
        let list: Vec<Drawing> = self
            .store
            .drawings(&self.chart.symbol.id)
            .and_then(|j| serde_json::from_str(j).ok())
            .unwrap_or_default();
        self.chart.set_drawings(list);
        self.drawings_saved = self.chart.drawings_changed;
    }

    pub(crate) fn save_drawings(&mut self, force: bool) {
        if self.chart.drawings_changed == self.drawings_saved {
            return;
        }
        // While a drawing is being dragged, wait for the drop.
        if !force && self.chart_ui.draw.selected.is_some() && self.last_save.elapsed() < Duration::from_secs(2) {
            return;
        }
        let json = if self.chart.drawings.is_empty() {
            None
        } else {
            serde_json::to_string(&self.chart.drawings).ok()
        };
        if let Err(e) = self.store.save_drawings(&self.chart.symbol.id, json.as_deref()) {
            crate::diag::crumb(format!("couldn't save drawings: {e}"));
        }
        self.drawings_saved = self.chart.drawings_changed;
        // Other charts on this symbol show the same drawings.
        let (id, list) = (self.chart.symbol.id.clone(), self.chart.drawings.clone());
        for s in self.slots.iter_mut().flatten() {
            if s.chart.symbol.id == id && s.chart.drawings != list {
                s.chart.set_drawings(list.clone());
                s.drawings_saved = s.chart.drawings_changed;
            }
        }
    }

    /// Changes the selected drawing (from its floating toolbar). Edits in
    /// quick succession, like dragging in the colour picker, are one undo step.
    pub fn edit_drawing(&mut self, id: u64, f: impl FnOnce(&mut Drawing)) {
        let fresh = self
            .last_style_edit
            .is_none_or(|(d, at)| d != id || at.elapsed() > Duration::from_secs(1));
        if fresh {
            self.chart.push_undo();
        }
        self.last_style_edit = Some((id, Instant::now()));
        if let Some(d) = self.chart.drawings.iter_mut().find(|d| d.id == id) {
            f(d);
            self.chart.drawings_changed += 1;
        }
    }

    pub fn delete_selected_drawing(&mut self) {
        if let Some(id) = self.chart_ui.draw.selected.take() {
            self.chart.push_undo();
            self.chart.drawings.retain(|d| d.id != id);
            self.chart.drawings_changed += 1;
        }
    }

    pub fn set_interval(&mut self, tf: Timeframe) {
        if tf == self.chart.tf {
            return;
        }
        crate::diag::crumb(format!("interval {}", tf.label()));
        let sym = self.chart.symbol.clone();
        self.chart.reset_series(sym, tf);
        self.open_chart();
    }

    pub fn intervals(&self) -> Vec<Timeframe> {
        let favs = self.store.favorites("interval");
        let list: Vec<&str> = if favs.is_empty() {
            DEFAULT_INTERVALS.to_vec()
        } else {
            favs.iter().map(|s| s.as_str()).collect()
        };
        let mut v: Vec<Timeframe> = list.iter().filter_map(|s| Timeframe::parse(s)).collect();
        v.sort_by_key(|t| t.millis().unwrap_or(i64::MAX));
        v
    }

    pub fn toggle_interval_favorite(&mut self, tf: Timeframe) {
        let mut list: Vec<String> = self.intervals().iter().map(|t| t.label()).collect();
        let key = tf.label();
        if let Some(i) = list.iter().position(|k| *k == key) {
            list.remove(i);
        } else {
            list.push(key);
        }
        if let Err(e) = self.store.set_favorites("interval", &list) {
            crate::diag::crumb(format!("couldn't save favourites: {e}"));
        }
    }

    pub fn toggle_favorite(&mut self, kind: &str, key: &str) {
        let mut list = self.store.favorites(kind).to_vec();
        if let Some(i) = list.iter().position(|k| k == key) {
            list.remove(i);
        } else {
            list.push(key.to_string());
        }
        if let Err(e) = self.store.set_favorites(kind, &list) {
            crate::diag::crumb(format!("couldn't save favourites: {e}"));
        }
    }

    /// The inputs and look a newly added indicator gets: the saved default
    /// if there is one, else TradingView's.
    pub fn study_defaults(&self, meta: &'static Meta) -> (Inputs, StudyLook) {
        match self
            .store
            .indicator_default(meta.id)
            .and_then(|j| serde_json::from_str::<StudyDefault>(j).ok())
        {
            Some(d) => (d.inputs.normalized(meta), d.look.fitted(meta)),
            None => (meta.default_inputs(), StudyLook::defaults(meta)),
        }
    }

    pub fn add_study(&mut self, meta: &'static Meta) {
        crate::diag::crumb(format!("add {}", meta.id));
        let (inputs, look) = self.study_defaults(meta);
        self.chart.add_study(meta, &inputs, look, None);
    }

    pub fn save_study_default(&mut self, meta: &'static Meta, inputs: &Inputs, look: &StudyLook) {
        let d = StudyDefault {
            inputs: inputs.clone(),
            look: look.clone(),
        };
        if let Ok(j) = serde_json::to_string(&d)
            && let Err(e) = self.store.set_indicator_default(meta.id, Some(&j))
        {
            crate::diag::crumb(format!("couldn't save default: {e}"));
        }
    }

    pub fn clear_study_default(&mut self, meta: &'static Meta) {
        let _ = self.store.set_indicator_default(meta.id, None);
    }

    pub fn set_chart_style(&mut self, style: ChartStyle) {
        if self.chart.style != style {
            self.chart.style = style;
            self.chart.apply_timezone();
            self.chart.style_changed();
            self.chart.touch();
        }
    }

    pub fn save_template(&mut self, kind: TemplateKind, name: &str) {
        let json = match kind {
            TemplateKind::Chart => serde_json::to_string(&self.chart.style),
            TemplateKind::Indicators => serde_json::to_string(&StudyTemplate {
                version: 1,
                studies: self.chart.saved_studies(),
                panes: self.chart.panes.clone(),
            }),
        };
        if let Ok(j) = json
            && let Err(e) = self.store.save_template(kind.key(), name, &j)
        {
            crate::diag::crumb(format!("couldn't save template: {e}"));
        }
    }

    pub fn delete_template(&mut self, kind: TemplateKind, name: &str) {
        let _ = self.store.delete_template(kind.key(), name);
    }

    pub fn set_default_template(&mut self, kind: TemplateKind, name: Option<&str>) {
        let _ = self.store.set_default_template(kind.key(), name);
    }

    pub fn apply_indicator_template(&mut self, name: &str) {
        let Some(t) = self.store.templates("indicators").iter().find(|t| t.name == name) else {
            return;
        };
        if let Ok(tpl) = serde_json::from_str::<StudyTemplate>(&t.json) {
            self.chart.apply_study_template(&tpl);
        }
    }

    pub(crate) fn chart_actions(&mut self, actions: Vec<Action>) {
        for a in actions {
            match a {
                Action::StudySettings(uid) => {
                    if let Some(s) = crate::dialogs::StudySettings::new(self, uid) {
                        self.modal = Some(Modal::Study(s));
                    }
                }
                Action::RemoveStudy(uid) => self.chart.remove_study(uid),
                Action::StudyMenu(uid, pos) => self.study_menu = Some((uid, pos)),
                Action::MoreHistory => self.hub.send(Request::More { chart: self.chart.id }),
                Action::RemoveCompare(i) => self.remove_compare(i),
                Action::ChartSettings => {
                    self.modal = Some(Modal::ChartSettings(crate::dialogs::ChartSettings::new(self)))
                }
            }
        }
    }

    /// The small menu from a legend row's "…" button.
    fn study_menu(&mut self, ctx: &egui::Context) {
        let Some((uid, pos)) = self.study_menu else { return };
        let Some(meta) = self.chart.studies.iter().find(|s| s.uid == uid).map(|s| s.meta()) else {
            self.study_menu = None;
            return;
        };
        let t = self.t();
        let mut close = false;
        let area = egui::Area::new(egui::Id::new("study-menu"))
            .order(egui::Order::Foreground)
            .fixed_pos(pos)
            .show(ctx, |ui| {
                egui::Frame::menu(ui.style()).show(ui, |ui| {
                    ui.set_min_width(190.0);
                    if ui.button("Settings…").clicked() {
                        self.chart_actions(vec![Action::StudySettings(uid)]);
                        close = true;
                    }
                    if ui.button("Save as default").clicked() {
                        if let Some(s) = self.chart.studies.iter().find(|s| s.uid == uid) {
                            let (i, l) = (s.inst.inputs.clone(), s.look.clone());
                            self.save_study_default(meta, &i, &l);
                        }
                        close = true;
                    }
                    if ui.button("Reset settings").clicked() {
                        let (i, l) = self.study_defaults(meta);
                        self.chart.set_inputs(uid, &i);
                        if let Some(s) = self.chart.study_mut(uid) {
                            s.look = l;
                        }
                        close = true;
                    }
                    ui.separator();
                    if ui.button(egui::RichText::new("Remove").color(t.neg)).clicked() {
                        self.chart.remove_study(uid);
                        close = true;
                    }
                });
            });
        let clicked_outside = ctx.input(|i| i.pointer.any_pressed())
            && !area.response.contains_pointer()
            && !ctx.input(|i| i.pointer.any_down() && area.response.hovered());
        if close || clicked_outside || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.study_menu = None;
        }
    }

    fn poll_data(&mut self) {
        for e in self.hub.poll() {
            match e {
                Event::Quote { id, last, prev } => {
                    self.quotes.insert(id, (last, prev));
                }
                Event::Fetched {
                    seq,
                    symbol,
                    tf,
                    result,
                } => self.fetched(seq, &symbol, tf, result),
                e @ (Event::Hotlist { .. } | Event::News { .. }) => self.side_event(e),
                Event::Search { seq, results } => {
                    if let Some(Modal::Symbol(s)) = &mut self.modal
                        && seq == self.search_seq
                    {
                        s.results = results;
                    }
                }
                e @ (Event::Bars { chart, .. }
                | Event::Tick { chart, .. }
                | Event::Status { chart, .. }
                | Event::Corporate { chart, .. }) => {
                    if chart == self.chart.id {
                        self.chart.apply(e);
                    } else if self.chart.owns_compare(chart) {
                        self.chart.apply_compare(e);
                    } else if let Some(c) = self
                        .slots
                        .iter_mut()
                        .flatten()
                        .map(|s| &mut s.chart)
                        .find(|c| c.id == chart || c.owns_compare(chart))
                    {
                        if c.id == chart {
                            c.apply(e);
                        } else {
                            c.apply_compare(e);
                        }
                    }
                }
            }
        }
    }

    /// One-off history arrived: for the screener, or a backtest over other
    /// symbols.
    fn fetched(&mut self, seq: u64, symbol: &str, _tf: Timeframe, result: Result<Vec<opentern_core::Bar>, String>) {
        self.screener_fetched(seq, symbol, result);
    }

    /// Saves the chart when it changed, at most every couple of seconds.
    fn autosave(&mut self, force: bool) {
        let version = self.layout_version();
        if version == self.saved {
            return;
        }
        if !force && self.last_save.elapsed() < Duration::from_secs(2) {
            return;
        }
        if let Ok(j) = serde_json::to_string(&self.layout_state())
            && let Err(e) = self.store.save_layout(self.layout_id, &self.layout_name, &j)
        {
            crate::diag::crumb(format!("couldn't save layout: {e}"));
        }
        self.saved = version;
        self.last_save = Instant::now();
    }

    /// TradingView's keys: start typing for symbol search, `/` for
    /// indicators, Alt+R to reset the chart.
    fn shortcuts(&mut self, ctx: &egui::Context) {
        if self.modal.is_some() || ctx.egui_wants_keyboard_input() {
            return;
        }
        let events = ctx.input(|i| i.events.clone());
        // Text typed with Alt/Ctrl/Cmd held belongs to a shortcut (Alt+A,
        // Alt+T, …), not to symbol search.
        let chord = ctx.input(|i| i.modifiers.alt || i.modifiers.command || i.modifiers.ctrl);
        for e in events {
            match e {
                egui::Event::Text(_) if chord => {}
                egui::Event::Text(s) if s == "/" => {
                    let has = !self.store.favorites("indicator").is_empty();
                    self.modal = Some(Modal::Indicators(crate::dialogs::Indicators::new(has)));
                }
                egui::Event::Text(s) if s.chars().all(|c| c.is_alphanumeric()) && !s.is_empty() => {
                    let first_digit = s.chars().next().is_some_and(|c| c.is_ascii_digit());
                    if first_digit {
                        // A number changes the interval, as on TradingView.
                        self.modal = Some(Modal::Interval(s.clone()));
                    } else {
                        self.modal = Some(Modal::Symbol(crate::dialogs::SymbolSearch::new(&s.to_uppercase())));
                    }
                }
                egui::Event::Key {
                    key: egui::Key::R,
                    pressed: true,
                    modifiers,
                    ..
                } if modifiers.alt => self.chart.reset_view(),
                egui::Event::Key {
                    key: egui::Key::F,
                    pressed: true,
                    modifiers,
                    ..
                } if modifiers.shift => {
                    let full = ctx.input(|i| i.viewport().fullscreen.unwrap_or(false));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(!full));
                }
                egui::Event::Key {
                    key: egui::Key::Delete | egui::Key::Backspace,
                    pressed: true,
                    ..
                } => self.delete_selected_drawing(),
                egui::Event::Key {
                    key: egui::Key::Escape,
                    pressed: true,
                    ..
                } => {
                    self.chart_ui.draw.cancel();
                    self.pick_tool(Tool::Cross);
                }
                egui::Event::Key {
                    key: egui::Key::Z,
                    pressed: true,
                    modifiers,
                    ..
                } if modifiers.command => {
                    if modifiers.shift {
                        self.chart.redo();
                    } else {
                        self.chart.undo();
                    }
                }
                egui::Event::Key {
                    key: egui::Key::Y,
                    pressed: true,
                    modifiers,
                    ..
                } if modifiers.command => {
                    self.chart.redo();
                }
                egui::Event::Key {
                    key: egui::Key::A,
                    pressed: true,
                    modifiers,
                    ..
                } if modifiers.alt => {
                    self.open_alert_dialog(None);
                }
                egui::Event::Key {
                    key: egui::Key::S,
                    pressed: true,
                    modifiers,
                    ..
                } if modifiers.alt => self.take_snapshot(crate::snapshot::Snap::Save),
                egui::Event::Key {
                    key: key @ (egui::Key::T | egui::Key::H | egui::Key::V | egui::Key::F),
                    pressed: true,
                    modifiers,
                    ..
                } if modifiers.alt => {
                    let tool = match key {
                        egui::Key::T => Tool::TrendLine,
                        egui::Key::H => Tool::HorizontalLine,
                        egui::Key::V => Tool::VerticalLine,
                        _ => Tool::FibRetracement,
                    };
                    self.pick_tool(tool);
                }
                egui::Event::Key {
                    key: egui::Key::ArrowLeft,
                    pressed: true,
                    ..
                } => self.chart.view.right -= 1.0,
                egui::Event::Key {
                    key: egui::Key::ArrowRight,
                    pressed: true,
                    ..
                } => self.chart.view.right += 1.0,
                _ => {}
            }
        }
    }

    pub(crate) fn frame(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        self.theme.tick(&ctx);
        let was_prompt = self.updater.prompt();
        if let Some(note) = self.updater.poll(&ctx) {
            let (title, body) = match note {
                Ok(m) => ("Update installed", m),
                Err(m) => ("Update failed", m),
            };
            self.notify(crate::icons::ph::ARROW_CIRCLE_UP, title, &body, None, false);
        }
        if !was_prompt && let crate::updater::Phase::Available(r) = &self.updater.phase {
            let body = format!("Opentern {} is ready to install from the rail.", r.version);
            self.notify(
                crate::icons::ph::ARROW_CIRCLE_UP,
                "Update available",
                &body,
                None,
                false,
            );
        }
        self.updater.launch_check(&ctx, self.store.settings().check_updates);
        self.poll_data();
        self.poll_scripts();
        self.refresh_strategies();
        self.poll_paper();
        self.poll_other_charts();
        self.poll_screener();
        self.poll_alerts();
        self.shortcuts(&ctx);
        self.shell(ui);
        self.drawing_bar(&ctx);
        self.modal(&ctx);
        self.alert_toasts(&ctx);
        self.note_toasts(&ctx);
        self.poll_snapshot(&ctx);
        self.study_menu(&ctx);
        self.autosave(false);
        self.save_drawings(false);
    }

    /// Scripts, strategy marks, paper lines and drawing saves for the charts
    /// that aren't active.
    fn poll_other_charts(&mut self) {
        for i in 0..self.slots.len() {
            if i == self.active {
                continue;
            }
            self.with_slot(i, |a| {
                a.poll_scripts();
                a.refresh_strategies();
                a.paper_lines();
                a.save_drawings(false);
            });
        }
    }

    /// After a panic in a frame: note it, and put the arrangement back to a
    /// known-good state. Gives up resetting after a few in a row so a bug
    /// that fires every frame doesn't spin.
    fn recover(&mut self) {
        self.panics += 1;
        crate::diag::note("RECOVERED", &format!("A frame panicked ({} so far).", self.panics));
        self.modal = None;
        self.study_menu = None;
        if self.panics <= 3 {
            self.layout = Arrangement::default();
        }
    }
}

#[cfg(test)]
impl App {
    /// What closing the window does, for tests (they can't build a Frame).
    pub(crate) fn on_exit_for_tests(&mut self) {
        self.autosave(true);
        self.each_chart(|a| a.save_drawings(true));
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // Frame pacing (vsync is off, see main.rs): while animating, don't
        // render faster than FRAME_CAP. Idle frames aren't affected.
        if let Some(last) = self.last_frame {
            let min = Duration::from_secs_f32(1.0 / FRAME_CAP);
            let since = last.elapsed();
            if since < min {
                std::thread::sleep(min - since);
            }
        }
        self.last_frame = Some(Instant::now());
        crate::diag::frame_begin();
        // Crash shield: a bug in one frame must never take the app down.
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.frame(ui)));
        crate::diag::frame_end();
        if result.is_err() {
            self.recover();
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.autosave(true);
        self.each_chart(|a| a.save_drawings(true));
        self.updater.on_exit();
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, "layout", &self.layout);
        self.autosave(true);
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        self.theme.current.bg.to_normalized_gamma_f32()
    }
}

/// True in tests: no file dialogs, no network, no launching other apps.
pub fn headless() -> bool {
    std::env::var_os("OPENTERN_HEADLESS").is_some()
}

/// Opens a file or folder with the system's default app.
pub fn open_path(path: &std::path::Path) {
    if headless() {
        return;
    }
    let _ = std::fs::create_dir_all(path.parent().unwrap_or(path));
    #[cfg(target_os = "macos")]
    let r = std::process::Command::new("open").arg(path).spawn();
    #[cfg(target_os = "windows")]
    let r = std::process::Command::new("explorer").arg(path).spawn();
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let r = std::process::Command::new("xdg-open").arg(path).spawn();
    if let Err(e) = r {
        crate::diag::crumb(format!("couldn't open {}: {e}", path.display()));
    }
}

/// Opens a web page in the default browser.
pub fn open_url(url: &str) {
    if headless() {
        return;
    }
    #[cfg(target_os = "macos")]
    let r = std::process::Command::new("open").arg(url).spawn();
    #[cfg(target_os = "windows")]
    let r = std::process::Command::new("explorer").arg(url).spawn();
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let r = std::process::Command::new("xdg-open").arg(url).spawn();
    if let Err(e) = r {
        crate::diag::crumb(format!("couldn't open link: {e}"));
    }
}

//! The window's arrangement, after TradingView's (traders already know it):
//! top bar, drawing toolbar on the left, chart in the middle, a widget panel
//! and icon rail on the right, the date-range bar under the chart, and a
//! bottom panel with tabs (screener, script editor, strategy report, paper
//! trading, alerts). See CLAUDE.md §4a.

use crate::app::App;
use crate::chart::draw::{Group, Tool};
use crate::chart::style::SeriesKind;
use crate::dialogs::{Modal, TemplateKind};
use crate::icons::ph;
use crate::motion;
use crate::theme::{self, Theme};
use crate::widgets as w;
use egui::{Align, Align2, CornerRadius, Id, Layout, Margin, Rect, Sense, Stroke, Ui, pos2, vec2};
use opentern_core::{Timeframe, Unit};
use serde::{Deserialize, Serialize};

pub const TOP_H: f32 = 44.0;
pub const TOOLBAR_W: f32 = 48.0;
pub const RAIL_W: f32 = 46.0;
pub const RANGE_H: f32 = 32.0;
pub const TABS_H: f32 = 32.0;
/// The gap between cards, and their corner radius.
pub const GAP: f32 = 5.0;
const CARD_R: f32 = 9.0;

/// Widgets the right rail opens in the widget panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Widget {
    Watchlist,
    Alerts,
    ObjectTree,
    Hotlists,
    Dom,
    Calendar,
    News,
    Notifications,
    Claude,
}

impl Widget {
    pub const ALL: [Widget; 9] = [
        Widget::Watchlist,
        Widget::Alerts,
        Widget::ObjectTree,
        Widget::Hotlists,
        Widget::Dom,
        Widget::Calendar,
        Widget::News,
        Widget::Notifications,
        Widget::Claude,
    ];

    fn icon(self) -> &'static str {
        match self {
            Widget::Watchlist => ph::LIST_BULLETS,
            Widget::Alerts => ph::ALARM,
            Widget::ObjectTree => ph::STACK,
            Widget::Hotlists => ph::FIRE,
            Widget::Dom => ph::ROWS,
            Widget::Calendar => ph::CALENDAR_DOTS,
            Widget::News => ph::NEWSPAPER,
            Widget::Notifications => ph::BELL_SIMPLE,
            Widget::Claude => ph::ROBOT,
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Widget::Watchlist => "Watchlist",
            Widget::Alerts => "Alerts",
            Widget::ObjectTree => "Object tree and data window",
            Widget::Hotlists => "Hotlists",
            Widget::Dom => "Depth of market",
            Widget::Calendar => "Calendar",
            Widget::News => "News",
            Widget::Notifications => "Notifications",
            Widget::Claude => "Claude",
        }
    }
}

/// Tabs of the bottom panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Tab {
    Screener,
    Editor,
    Report,
    Lab,
    Paper,
    AlertLog,
}

impl Tab {
    pub const ALL: [Tab; 6] = [
        Tab::Screener,
        Tab::Editor,
        Tab::Report,
        Tab::Lab,
        Tab::Paper,
        Tab::AlertLog,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Tab::Screener => "Screener",
            Tab::Editor => "Script Editor",
            Tab::Report => "Strategy Report",
            Tab::Lab => "Strategy Lab",
            Tab::Paper => "Paper Trading",
            Tab::AlertLog => "Alerts log",
        }
    }
}

/// Everything about the arrangement that's remembered between launches.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Arrangement {
    pub widget: Option<Widget>,
    pub widget_w: f32,
    pub tab: Option<Tab>,
    pub panel_h: f32,
    pub tool: Tool,
    /// The tool each toolbar group shows (the last one picked from it).
    pub group_tools: Vec<(Group, Tool)>,
    pub magnet: bool,
    pub stay_drawing: bool,
    pub lock_drawings: bool,
    pub hide_drawings: bool,
    /// The bottom panel takes nearly all the height (the chart keeps a sliver).
    pub panel_max: bool,
    /// The drawing toolbar on the left is showing.
    pub toolbar: bool,
}

impl Default for Arrangement {
    fn default() -> Self {
        Arrangement {
            widget: Some(Widget::Watchlist),
            widget_w: 300.0,
            tab: None,
            panel_h: 280.0,
            tool: Tool::Cross,
            group_tools: Vec::new(),
            magnet: false,
            stay_drawing: false,
            lock_drawings: false,
            hide_drawings: false,
            panel_max: false,
            toolbar: true,
        }
    }
}

impl App {
    /// Lays out the whole window. Panel order matters: the ones added first
    /// take the full width or height.
    pub(crate) fn shell(&mut self, ui: &mut Ui) {
        let t = self.t();
        egui::Panel::top("top-bar")
            .exact_size(TOP_H)
            .resizable(false)
            .show_separator_line(false)
            .frame(egui::Frame::new().fill(t.sidebar).inner_margin(Margin::symmetric(8, 0)))
            .show(ui, |ui| self.top_bar(ui, &t));

        egui::Panel::right("rail")
            .exact_size(RAIL_W)
            .resizable(false)
            .show_separator_line(false)
            .frame(egui::Frame::new().fill(t.sidebar).inner_margin(Margin::symmetric(0, 6)))
            .show(ui, |ui| self.rail(ui, &t));

        // The widget panel slides in and out; its width is dragged by its
        // left edge.
        // The chart, the widget panel and the open bottom panel are cards
        // on the darker frame, with a small gap between them: the regions
        // read apart at a glance (CLAUDE.md §4a).
        let mut cards: Vec<Rect> = Vec::new();
        let open = self.layout.widget.is_some();
        let k = motion::toggle(ui.ctx(), Id::new("widget-open"), open, motion::STANDARD);
        if k > 0.001 {
            let width = self.layout.widget_w * k;
            egui::Panel::right("widget-panel")
                .exact_size(width)
                .resizable(false)
                .show_separator_line(false)
                .frame(egui::Frame::new().fill(t.sidebar).inner_margin(Margin {
                    left: GAP as i8,
                    right: 0,
                    top: GAP as i8,
                    bottom: GAP as i8,
                }))
                .show(ui, |ui| {
                    cards.push(ui.max_rect());
                    ui.set_opacity(k);
                    self.widget_panel(ui, &t)
                });
        }

        // Bottom panel: the tab strip always shows; the content slides up
        // when a tab is open.
        let k = motion::toggle(
            ui.ctx(),
            Id::new("panel-open"),
            self.layout.tab.is_some(),
            motion::STANDARD,
        );
        let panel_h = if self.layout.panel_max {
            (ui.ctx().content_rect().height() - TOP_H - RANGE_H - TABS_H - 110.0).max(self.layout.panel_h)
        } else {
            self.layout.panel_h
        };
        egui::Panel::bottom("bottom-panel")
            .exact_size(TABS_H + panel_h * k)
            .resizable(false)
            .show_separator_line(false)
            .frame(egui::Frame::new().fill(t.sidebar))
            .show(ui, |ui| {
                if let Some(r) = self.bottom_panel(ui, &t, k) {
                    cards.push(r);
                }
            });

        egui::Panel::bottom("range-bar")
            .exact_size(RANGE_H)
            .resizable(false)
            .show_separator_line(false)
            .frame(egui::Frame::new().fill(t.sidebar).inner_margin(Margin::symmetric(8, 0)))
            .show(ui, |ui| self.range_bar(ui, &t));

        if self.layout.toolbar {
            egui::Panel::left("drawing-toolbar")
                .exact_size(TOOLBAR_W)
                .resizable(false)
                .show_separator_line(false)
                .frame(egui::Frame::new().fill(t.sidebar).inner_margin(Margin::symmetric(0, 6)))
                .show(ui, |ui| self.drawing_toolbar(ui, &t));
        }

        egui::CentralPanel::no_frame()
            .frame(egui::Frame::new().fill(t.sidebar).inner_margin(Margin {
                left: if self.layout.toolbar { 0 } else { GAP as i8 },
                right: if self.layout.widget.is_some() { 0 } else { GAP as i8 },
                top: GAP as i8,
                bottom: 0,
            }))
            .show(ui, |ui| {
                cards.push(ui.max_rect());
                if self.settings.is_some() {
                    self.settings_page(ui, &t);
                } else {
                    self.chart_grid(ui, &t)
                }
            });

        // Round the cards' corners and edge them, drawn last so they sit
        // over what the panels painted (and under popups and dialogs).
        let p = ui
            .ctx()
            .layer_painter(egui::LayerId::new(egui::Order::Middle, Id::new("cards")));
        for r in cards {
            if r.width() > 2.0 * CARD_R && r.height() > 2.0 * CARD_R {
                round_corners(&p, r, CARD_R, t.sidebar);
                p.rect_stroke(
                    r,
                    CornerRadius::same(CARD_R as u8),
                    Stroke::new(1.0, motion::with_alpha(t.border, 0.9)),
                    egui::StrokeKind::Inside,
                );
            }
        }
    }

    // ------------------------------------------------------------ top bar

    fn top_bar(&mut self, ui: &mut Ui, t: &Theme) {
        // Narrow windows drop the labels next to icons first.
        let compact = ui.available_width() < 1240.0;
        let label = |s: &'static str| if compact { "" } else { s };
        // The right-hand group is laid out first, so the left one can only
        // take the space that's left and never runs underneath it.
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            let snap = w::bar_button(ui, t, Some(ph::CAMERA), "", "Take a snapshot (Alt+S)", false);
            egui::Popup::menu(&snap).show(|ui| {
                ui.set_width(220.0);
                ui.spacing_mut().item_spacing.y = 1.0;
                if w::menu_item(ui, t, Some(ph::DOWNLOAD_SIMPLE), "Download image", "", false).clicked() {
                    self.take_snapshot(crate::snapshot::Snap::Save);
                    ui.close();
                }
                if w::menu_item(ui, t, Some(ph::COPY), "Copy image", "", false).clicked() {
                    self.take_snapshot(crate::snapshot::Snap::Copy);
                    ui.close();
                }
                if w::menu_item(ui, t, Some(ph::FOLDER_OPEN), "Open snapshots folder", "", false).clicked() {
                    let dir = self.store.dir().join("snapshots");
                    let _ = std::fs::create_dir_all(&dir);
                    crate::app::open_path(&dir.join("."));
                    ui.close();
                }
            });
            let full = ui.ctx().input(|i| i.viewport().fullscreen.unwrap_or(false));
            let glyph = if full { ph::CORNERS_IN } else { ph::CORNERS_OUT };
            if w::bar_button(ui, t, Some(glyph), "", "Fullscreen mode (Shift F)", full).clicked() {
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Fullscreen(!full));
            }
            w::vdivider(ui, t);
            // The layout's name with a caret, as on TradingView; a folder
            // icon when the bar is short of room.
            let name = elide(&self.layout_name, 18);
            let resp = if compact {
                w::bar_button(ui, t, Some(ph::FOLDER_SIMPLE), "", "Layouts", false)
            } else {
                w::bar_button(ui, t, None, &format!("{name}  {}", ph::CARET_DOWN), "Layouts", false)
            };
            self.still_open(ui, &resp, "layout");
            egui::Popup::menu(&resp)
                .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                .show(|ui| self.layout_menu(ui, t));
            let resp = w::bar_button(ui, t, Some(ph::LAYOUT), "", "Select layout", self.chart_count() > 1);
            egui::Popup::menu(&resp)
                .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                .show(|ui| self.grid_menu(ui, t));
            w::vdivider(ui, t);

            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                ui.set_clip_rect(ui.max_rect().intersect(ui.clip_rect()));
                ui.spacing_mut().item_spacing.x = 2.0;
                let open = self.settings.is_some();
                let menu = w::bar_button(ui, t, Some(ph::LIST), "", "Settings", open);
                if menu.clicked() {
                    if open {
                        self.settings = None;
                    } else {
                        self.open_settings(crate::settings::Cat::Appearance);
                    }
                }
                w::vdivider(ui, t);

                // Symbol search: TradingView opens it when you start typing.
                let ticker = self.chart.symbol.ticker().to_string();
                if w::bar_button(
                    ui,
                    t,
                    Some(ph::MAGNIFYING_GLASS),
                    &ticker,
                    "Symbol search (start typing anywhere)",
                    false,
                )
                .clicked()
                {
                    self.modal = Some(Modal::Symbol(crate::dialogs::SymbolSearch::new("")));
                }
                if w::bar_button(ui, t, Some(ph::PLUS_CIRCLE), "", "Compare or add symbol", false).clicked() {
                    self.modal = Some(Modal::Symbol(crate::dialogs::SymbolSearch::for_compare()));
                }
                w::vdivider(ui, t);

                // Favourite intervals inline, the rest in a menu.
                for tf in self.intervals() {
                    let active = self.chart.tf == tf;
                    if w::bar_button(ui, t, None, &tf.label(), "", active).clicked() {
                        self.set_interval_synced(tf);
                    }
                }
                if !self.intervals().contains(&self.chart.tf) {
                    w::bar_button(ui, t, None, &self.chart.tf.label(), "", true);
                }
                self.interval_menu(ui, t);
                w::vdivider(ui, t);

                self.chart_type_menu(ui, t);
                if w::bar_button(
                    ui,
                    t,
                    Some(ph::FUNCTION),
                    "Indicators",
                    "Indicators, metrics and strategies (/)",
                    false,
                )
                .clicked()
                {
                    let has = !self.store.favorites("indicator").is_empty();
                    self.modal = Some(Modal::Indicators(crate::dialogs::Indicators::new(has)));
                }
                self.templates_menu(ui, t);
                w::vdivider(ui, t);
                if w::bar_button(ui, t, Some(ph::ALARM), label("Alert"), "Create alert (Alt+A)", false).clicked() {
                    self.open_alert_dialog(None);
                }
                if w::bar_button(
                    ui,
                    t,
                    Some(ph::REWIND),
                    label("Replay"),
                    "Bar replay",
                    self.chart.replay.is_some(),
                )
                .clicked()
                {
                    self.chart.toggle_replay();
                }
                w::vdivider(ui, t);
                let undo = egui::Button::new(ph::ARROW_U_UP_LEFT).frame(false);
                if ui
                    .add_enabled(self.chart.can_undo(), undo)
                    .on_hover_text(concat!("Undo (", shortcut!("Z"), ")"))
                    .clicked()
                {
                    self.chart.undo();
                }
                let redo = egui::Button::new(ph::ARROW_U_UP_RIGHT).frame(false);
                if ui
                    .add_enabled(self.chart.can_redo(), redo)
                    .on_hover_text(concat!("Redo (", shortcut!("Y"), ")"))
                    .clicked()
                {
                    self.chart.redo();
                }
            });
        });
    }

    /// The "more intervals" menu: every standard interval by unit, a star
    /// beside each to pin it to the top bar, and a field for a custom one
    /// ("7m", "3h").
    fn interval_menu(&mut self, ui: &mut Ui, t: &Theme) {
        let resp = w::bar_button(ui, t, Some(ph::CARET_DOWN), "", "More intervals", false);
        self.still_open(ui, &resp, "interval");
        egui::Popup::menu(&resp)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .show(|ui| {
                ui.set_width(196.0);
                ui.spacing_mut().item_spacing.y = 1.0;
                let favs = self.intervals();
                let mut last = None;
                for tf in Timeframe::STANDARD {
                    let unit = match tf.unit {
                        Unit::Second | Unit::Minute => "MINUTES",
                        Unit::Hour => "HOURS",
                        _ => "DAYS",
                    };
                    if last != Some(unit) {
                        if last.is_some() {
                            w::menu_rule(ui, t);
                        }
                        w::menu_heading(ui, t, unit);
                        last = Some(unit);
                    }
                    let fav = favs.contains(tf);
                    let (pick, star) = interval_row(ui, t, *tf, self.chart.tf == *tf, fav);
                    if star {
                        self.toggle_interval_favorite(*tf);
                    } else if pick {
                        self.set_interval_synced(*tf);
                        ui.close();
                    }
                }
                w::menu_rule(ui, t);
                let id = Id::new("custom-interval");
                let mut text: String = ui.data(|d| d.get_temp(id)).unwrap_or_default();
                let r = ui.add(
                    w::field(&mut text)
                        .hint_text("Custom: 7m, 3h, 2D")
                        .desired_width(f32::INFINITY),
                );
                if r.lost_focus()
                    && ui.input(|i| i.key_pressed(egui::Key::Enter))
                    && let Some(tf) = Timeframe::parse(&text)
                {
                    self.set_interval_synced(tf);
                    text.clear();
                    ui.close();
                }
                ui.data_mut(|d| d.insert_temp(id, text));
            });
    }

    fn chart_type_menu(&mut self, ui: &mut Ui, t: &Theme) {
        let kind = self.chart.style.kind;
        let resp = w::bar_icon_button(ui, t, "Bars style", |p, r, c| {
            crate::chart::style::paint_kind_icon(p, r, kind, c)
        });
        self.still_open(ui, &resp, "type");
        egui::Popup::menu(&resp).show(|ui| {
            ui.set_width(210.0);
            ui.spacing_mut().item_spacing.y = 1.0;
            for k in SeriesKind::ALL {
                if k == SeriesKind::HeikinAshi {
                    w::menu_rule(ui, t);
                }
                let on = self.chart.style.kind == k;
                let paint =
                    |p: &egui::Painter, r: Rect, c: egui::Color32| crate::chart::style::paint_kind_icon(p, r, k, c);
                if w::menu_item_painted(ui, t, paint, k.name(), on).clicked() {
                    let mut s = self.chart.style.clone();
                    s.kind = k;
                    self.set_chart_style(s);
                    ui.close();
                }
            }
        });
    }

    /// Indicator templates: save the current set, apply or delete saved ones.
    fn templates_menu(&mut self, ui: &mut Ui, t: &Theme) {
        let resp = w::bar_button(ui, t, Some(ph::SQUARES_FOUR), "", "Indicator templates", false);
        self.still_open(ui, &resp, "templates");
        egui::Popup::menu(&resp).show(|ui| {
            ui.set_width(250.0);
            ui.spacing_mut().item_spacing.y = 1.0;
            if w::menu_item(ui, t, Some(ph::FLOPPY_DISK), "Save indicator template…", "", false).clicked() {
                self.modal = Some(Modal::SaveTemplate {
                    kind: TemplateKind::Indicators,
                    name: String::new(),
                });
                ui.close();
            }
            let list = self.store.templates("indicators").to_vec();
            w::menu_rule(ui, t);
            if list.is_empty() {
                ui.horizontal(|ui| {
                    ui.add_space(10.0);
                    ui.label(
                        egui::RichText::new("Your saved templates show up here.")
                            .font(theme::regular(12.5))
                            .color(t.text3),
                    );
                });
                ui.add_space(4.0);
            } else {
                w::menu_heading(ui, t, "APPLY");
            }
            for tpl in list {
                let n = tpl.name.clone();
                let resp = w::menu_item(ui, t, Some(ph::SQUARES_FOUR), &n, "", false);
                let bin = Rect::from_center_size(resp.rect.right_center() - vec2(16.0, 0.0), vec2(24.0, 24.0));
                let b = ui.interact(bin, resp.id.with("del"), Sense::click());
                if resp.hovered() || b.hovered() {
                    ui.painter().text(
                        bin.center(),
                        Align2::CENTER_CENTER,
                        ph::TRASH,
                        theme::regular(14.0),
                        if b.hovered() { t.neg } else { t.text3 },
                    );
                }
                if b.on_hover_text("Delete template").clicked() {
                    self.delete_template(TemplateKind::Indicators, &n);
                } else if resp.clicked() {
                    self.apply_indicator_template(&n);
                    ui.close();
                }
            }
        });
    }

    // ---------------------------------------------------- drawing toolbar

    fn drawing_toolbar(&mut self, ui: &mut Ui, t: &Theme) {
        egui::ScrollArea::vertical()
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
            .show(ui, |ui| self.drawing_tools(ui, t));
    }

    fn drawing_tools(&mut self, ui: &mut Ui, t: &Theme) {
        ui.vertical_centered(|ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            for group in Group::ALL {
                self.tool_group(ui, t, group);
            }
            w::hdivider(ui, t, TOOLBAR_W);
            let measure = Tool::Measure;
            if w::tool_button(
                ui,
                t,
                ph::RULER,
                "Measure (or Shift+drag)",
                self.layout.tool == measure,
                36.0,
            )
            .clicked()
            {
                self.pick_tool(measure);
            }
            if w::tool_button(
                ui,
                t,
                ph::MAGNIFYING_GLASS_PLUS,
                "Zoom in (drag a range)",
                self.layout.tool == Tool::Zoom,
                36.0,
            )
            .clicked()
            {
                self.pick_tool(Tool::Zoom);
            }
            w::hdivider(ui, t, TOOLBAR_W);
            let l = &mut self.layout;
            if w::tool_button(
                ui,
                t,
                ph::MAGNET,
                "Magnet mode: snap to open, high, low, close",
                l.magnet,
                36.0,
            )
            .clicked()
            {
                l.magnet = !l.magnet;
            }
            if w::tool_button(ui, t, ph::PENCIL_SIMPLE, "Stay in drawing mode", l.stay_drawing, 36.0).clicked() {
                l.stay_drawing = !l.stay_drawing;
            }
            let lock = if l.lock_drawings {
                ph::LOCK_SIMPLE
            } else {
                ph::LOCK_SIMPLE_OPEN
            };
            if w::tool_button(ui, t, lock, "Lock all drawings", l.lock_drawings, 36.0).clicked() {
                l.lock_drawings = !l.lock_drawings;
            }
            let eye = if l.hide_drawings { ph::EYE_SLASH } else { ph::EYE };
            if w::tool_button(ui, t, eye, "Hide all drawings", l.hide_drawings, 36.0).clicked() {
                l.hide_drawings = !l.hide_drawings;
            }
            w::hdivider(ui, t, TOOLBAR_W);
            let resp = w::tool_button(ui, t, ph::TRASH, "Remove drawings or indicators", false, 36.0);
            egui::Popup::menu(&resp).show(|ui| {
                ui.set_width(220.0);
                ui.spacing_mut().item_spacing.y = 1.0;
                let n = self.chart.drawings.len();
                let label = format!("Remove {n} drawing{}", if n == 1 { "" } else { "s" });
                if w::menu_row(ui, t, Some(ph::PENCIL_SIMPLE_SLASH), &label, "", false, n > 0).clicked() {
                    self.chart.push_undo();
                    self.chart.drawings.clear();
                    self.chart.drawings_changed += 1;
                    ui.close();
                }
                let k = self.chart.studies.len();
                let label = format!("Remove {k} indicator{}", if k == 1 { "" } else { "s" });
                if w::menu_row(ui, t, Some(ph::FUNCTION), &label, "", false, k > 0).clicked() {
                    let ids: Vec<u64> = self.chart.studies.iter().map(|s| s.uid).collect();
                    for id in ids {
                        self.chart.remove_study(id);
                    }
                    ui.close();
                }
            });
        });
    }

    /// The tool a group shows: the last one picked from it.
    fn group_tool(&self, g: Group) -> Tool {
        self.layout
            .group_tools
            .iter()
            .find(|(x, _)| *x == g)
            .map(|(_, t)| *t)
            .unwrap_or(g.tools()[0])
    }

    pub(crate) fn pick_tool(&mut self, tool: Tool) {
        let g = tool.group();
        self.layout.group_tools.retain(|(x, _)| *x != g);
        self.layout.group_tools.push((g, tool));
        self.layout.tool = tool;
        self.chart_ui.draw.cancel();
    }

    /// One toolbar button with a flyout of the group's tools, as on
    /// TradingView: click uses the shown tool, the small arrow opens the list.
    fn tool_group(&mut self, ui: &mut Ui, t: &Theme, g: Group) {
        let shown = self.group_tool(g);
        let active =
            self.layout.tool.group() == g && self.layout.tool != Tool::DatePriceRange && self.layout.tool != Tool::Zoom;
        let resp = w::tool_button(
            ui,
            t,
            shown.icon(),
            &format!("{} ({})", shown.name(), g.name()),
            active,
            36.0,
        );
        let r = resp.rect;
        let caret = Rect::from_min_max(pos2(r.right() - 9.0, r.bottom() - 9.0), r.max);
        let more = g.tools().len() > 1;
        if more {
            ui.painter().text(
                caret.center(),
                Align2::CENTER_CENTER,
                ph::CARET_DOWN,
                theme::regular(8.0),
                t.text3,
            );
        }
        let on_caret = resp
            .interact_pointer_pos()
            .is_some_and(|p| caret.expand(2.0).contains(p));
        let open_id = Id::new(("tool-flyout", g));
        if resp.clicked() && !(more && on_caret) {
            self.pick_tool(shown);
        }
        if more && (resp.secondary_clicked() || (resp.clicked() && on_caret)) {
            egui::Popup::open_id(ui.ctx(), open_id);
        }
        egui::Popup::from_response(&resp)
            .id(open_id)
            .open_memory(None)
            .align(egui::RectAlign::RIGHT_START)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClick)
            .show(|ui| {
                ui.set_width(230.0);
                ui.spacing_mut().item_spacing.y = 1.0;
                w::menu_heading(ui, t, &g.name().to_uppercase());
                for tool in g.tools() {
                    let on = self.layout.tool == *tool;
                    if w::menu_item(ui, t, Some(tool.icon()), tool.name(), "", on).clicked() {
                        self.pick_tool(*tool);
                    }
                }
            });
    }

    // --------------------------------------------------------------- rail

    fn rail(&mut self, ui: &mut Ui, t: &Theme) {
        ui.vertical_centered(|ui| {
            ui.spacing_mut().item_spacing.y = 4.0;
            for wd in Widget::ALL {
                let active = self.layout.widget == Some(wd);
                let resp = if wd == Widget::Claude {
                    crate::claude::mark::rail_button(ui, t, wd.title(), active, 34.0)
                } else {
                    w::tool_button(ui, t, wd.icon(), wd.title(), active, 34.0)
                };
                if wd == Widget::Notifications && self.notes.unread > 0 {
                    let dot = resp.rect.right_top() + vec2(-8.0, 8.0);
                    ui.painter().circle_filled(dot, 3.5, t.accent);
                }
                if resp.clicked() {
                    self.layout.widget = if active { None } else { Some(wd) };
                }
            }
            ui.with_layout(Layout::bottom_up(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.y = 4.0;
                let dark = t.dark;
                let tip = if dark {
                    "Switch to light theme"
                } else {
                    "Switch to dark theme"
                };
                if w::tool_button(ui, t, if dark { ph::SUN } else { ph::MOON }, tip, false, 34.0).clicked() {
                    self.toggle_dark(ui.ctx());
                }
                if self.updater.prompt() {
                    self.update_button(ui, t);
                }
            });
        });
    }

    /// The quiet update prompt: an icon with an accent dot. Click to update,
    /// then click again to restart. Never installs on its own.
    fn update_button(&mut self, ui: &mut Ui, t: &Theme) {
        use crate::updater::Phase;
        let tip = match &self.updater.phase {
            Phase::Available(r) => format!("Opentern {} is available. Click to update.", r.version),
            Phase::Manual(r, why) => format!("Opentern {} is available. {why}.", r.version),
            Phase::Installing(_) => format!("Updating… {:.0}%", self.updater.progress * 100.0),
            Phase::Ready(r) => format!("Opentern {} is installed. Click to restart.", r.version),
            _ => String::new(),
        };
        let resp = w::tool_button(ui, t, ph::ARROW_CIRCLE_UP, &tip, false, 34.0);
        let dot = resp.rect.right_top() + vec2(-8.0, 8.0);
        ui.painter().circle_filled(dot, 3.5, t.accent);
        if resp.clicked() {
            self.update_action(ui.ctx());
        }
    }

    // ------------------------------------------------------- widget panel

    fn widget_panel(&mut self, ui: &mut Ui, t: &Theme) {
        let full = ui.max_rect();
        ui.painter().rect_filled(full, CornerRadius::ZERO, t.bg);
        // The gap to the chart's left is the drag handle.
        let d = w::drag_handle(
            ui,
            t,
            Rect::from_min_max(
                pos2(full.left() - GAP, full.top()),
                pos2(full.left() + 1.0, full.bottom()),
            ),
            "widget-resize",
            true,
        );
        if d != 0.0 {
            self.layout.widget_w = (self.layout.widget_w - d).clamp(220.0, 560.0);
        }
        let Some(widget) = self.layout.widget else { return };
        // A child of exactly the panel's size, clipped: content that's too
        // wide (a long indicator name) must never widen the panel, or it
        // slides under the chart card.
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(full));
        child.set_clip_rect(full.intersect(ui.clip_rect()));
        let ui = &mut child;
        egui::Frame::new()
            .inner_margin(Margin::symmetric(12, 10))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                if !matches!(widget, Widget::Watchlist | Widget::Alerts | Widget::Claude) {
                    ui.horizontal(|ui| {
                        ui.set_height(26.0);
                        ui.label(egui::RichText::new(widget.title()).font(theme::semibold(14.0)));
                        if widget == Widget::Notifications && !self.notes.list.is_empty() {
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                if small_button(ui, t, "Clear all", false).clicked() {
                                    self.notes.list.clear();
                                }
                            });
                        }
                    });
                    ui.add_space(6.0);
                }
                match widget {
                    Widget::Watchlist => self.watchlist_widget(ui, t),
                    Widget::ObjectTree => self.objects_widget(ui, t),
                    Widget::Alerts => self.alerts_widget(ui, t),
                    Widget::Notifications => self.notifications_widget(ui, t),
                    Widget::Claude => self.claude_widget(ui, t),
                    Widget::Hotlists => self.hotlists_widget(ui, t),
                    Widget::Dom => self.dom_widget(ui, t),
                    Widget::News => self.news_widget(ui, t),
                    Widget::Calendar => self.calendar_widget(ui, t),
                }
            });
    }

    // --------------------------------------------------------- range bar

    fn range_bar(&mut self, ui: &mut Ui, t: &Theme) {
        let roomy = ui.available_width() > 760.0;
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            let auto = self.chart.panes.first().is_some_and(|p| p.scale.auto);
            if small_button(ui, t, "auto", auto)
                .on_hover_text("Auto (fits data to screen)")
                .clicked()
                && let Some(p) = self.chart.panes.first_mut()
            {
                p.scale.auto = !p.scale.auto;
            }
            if small_button(ui, t, "log", self.chart.log)
                .on_hover_text("Log scale")
                .clicked()
            {
                self.chart.log = !self.chart.log;
                self.chart.percent = false;
                if let Some(p) = self.chart.panes.first_mut() {
                    p.scale.auto = true;
                }
                self.chart.touch();
            }
            let has_divs = self.chart.corporate.as_ref().is_some_and(|c| !c.dividends.is_empty());
            if has_divs
                && small_button(ui, t, "ADJ", self.chart.adj)
                    .on_hover_text("Adjust prices for dividends")
                    .clicked()
            {
                let on = !self.chart.adj;
                self.chart.set_adj(on);
            }
            if small_button(ui, t, "%", self.chart.percent)
                .on_hover_text("Percent scale")
                .clicked()
            {
                self.chart.percent = !self.chart.percent;
                self.chart.log = false;
            }
            if roomy {
                w::vdivider(ui, t);
                // Minutes only, repainting on the minute: a seconds clock
                // would cost a frame every second while idle.
                let now = jiff::Timestamp::now().to_zoned(self.chart.tz.clone());
                let clock = format!("{}  {}", now.strftime("%H:%M"), now.strftime("UTC%:z"));
                small_button(ui, t, &clock, false).on_hover_text("Chart timezone (Chart settings → Symbol)");
                let secs = 60 - (now.second() as u64).min(59);
                ui.ctx().request_repaint_after(std::time::Duration::from_secs(secs));
            }
            w::vdivider(ui, t);
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                ui.set_clip_rect(ui.max_rect().intersect(ui.clip_rect()));
                ui.spacing_mut().item_spacing.x = 0.0;
                // Under the toolbar's column: hide or show the drawing toolbar.
                // The ranges then start where the chart does.
                let shown = self.layout.toolbar;
                let (glyph, tip) = if shown {
                    (ph::CARET_DOUBLE_LEFT, "Hide the drawing toolbar")
                } else {
                    (ph::CARET_DOUBLE_RIGHT, "Show the drawing toolbar")
                };
                let col = TOOLBAR_W - 8.0;
                ui.add_space(((col - 26.0) / 2.0).max(0.0));
                if w::tool_button(ui, t, glyph, tip, false, 26.0).clicked() {
                    self.layout.toolbar = !shown;
                }
                ui.add_space(((col - 26.0) / 2.0).max(0.0) + if shown { 0.0 } else { 4.0 });
                // TradingView's date ranges: each picks an interval and
                // fits that much history on screen.
                for (label, tf, days) in [
                    ("1D", "1m", 1),
                    ("5D", "5m", 5),
                    ("1M", "30m", 30),
                    ("3M", "1h", 91),
                    ("6M", "2h", 182),
                    ("YTD", "1D", 0),
                    ("1Y", "1D", 365),
                    ("5Y", "1W", 1826),
                    ("All", "1M", -1),
                ] {
                    if small_button(ui, t, label, false).clicked()
                        && let Some(tf) = Timeframe::parse(tf)
                    {
                        self.set_interval_synced(tf);
                        self.chart.fit_days = Some(days);
                    }
                }
            });
        });
    }

    // ------------------------------------------------------ bottom panel

    /// The tab strip and, when a tab is open, its body. Returns the body's
    /// card rect.
    fn bottom_panel(&mut self, ui: &mut Ui, t: &Theme, k: f32) -> Option<Rect> {
        let full = ui.max_rect();
        let mut card = None;
        if k > 0.0 {
            let d = w::drag_handle(
                ui,
                t,
                Rect::from_min_max(
                    pos2(full.left(), full.top() - 4.0),
                    pos2(full.right(), full.top() + 4.0),
                ),
                "panel-resize",
                false,
            );
            if d != 0.0 {
                if self.layout.panel_max {
                    // Dragging a maximized panel starts from its full height.
                    self.layout.panel_max = false;
                    self.layout.panel_h = full.height() - TABS_H;
                }
                self.layout.panel_h = (self.layout.panel_h - d).clamp(140.0, 900.0);
            }
        }
        let strip = Rect::from_min_size(full.min, vec2(full.width(), TABS_H));
        ui.scope_builder(egui::UiBuilder::new().max_rect(strip.shrink2(vec2(8.0, 0.0))), |ui| {
            ui.horizontal_centered(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                for tab in Tab::ALL {
                    let active = self.layout.tab == Some(tab);
                    if small_button(ui, t, tab.title(), active).clicked() {
                        self.layout.tab = if active { None } else { Some(tab) };
                    }
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let open = self.layout.tab.is_some();
                    let glyph = if open { ph::CARET_DOWN } else { ph::CARET_UP };
                    let tip = if open { "Collapse panel" } else { "Open panel" };
                    if w::tool_button(ui, t, glyph, tip, false, 26.0).clicked() {
                        self.layout.tab = if open { None } else { Some(self.last_tab) };
                    }
                    let max = self.layout.panel_max;
                    let (g, tip) = if max {
                        (ph::ARROWS_IN_SIMPLE, "Restore panel")
                    } else {
                        (ph::ARROWS_OUT_SIMPLE, "Maximize panel")
                    };
                    if w::tool_button(ui, t, g, tip, max, 26.0).clicked() {
                        self.layout.panel_max = !max;
                        if !max && self.layout.tab.is_none() {
                            self.layout.tab = Some(self.last_tab);
                        }
                    }
                });
            });
        });
        if let Some(tab) = self.layout.tab {
            self.last_tab = tab;
        }
        if k > 0.01 {
            let body = Rect::from_min_max(
                pos2(full.left() + GAP, strip.bottom()),
                pos2(
                    full.right() - if self.layout.widget.is_some() { 0.0 } else { GAP },
                    full.bottom() - GAP,
                ),
            );
            ui.painter().rect_filled(body, CornerRadius::ZERO, t.bg);
            card = Some(body);
            // A child that can't grow the panel: content taller than the
            // panel is clipped (and scrolls where it's long), never pushes
            // the tab strip under the range bar.
            // The editor runs edge to edge, like an IDE; the rest get a margin.
            let inner = if self.last_tab == Tab::Editor {
                body
            } else {
                body.shrink(12.0)
            };
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(inner));
            child.set_clip_rect(body.intersect(ui.clip_rect()));
            {
                let ui = &mut child;
                ui.set_opacity(k);
                match self.last_tab {
                    Tab::Screener => self.screener_tab(ui, t),
                    Tab::Editor => self.editor_tab(ui, t),
                    Tab::Report => self.report_tab(ui, t),
                    Tab::Lab => self.lab_tab(ui, t),
                    Tab::Paper => self.paper_tab(ui, t),
                    Tab::AlertLog => self.alert_log_tab(ui, t),
                }
            }
        }
        card
    }

    // ------------------------------------------------------------- chart

    /// One chart in `ui`'s rect (the active one, or a slot swapped in).
    pub(crate) fn chart_area_in(&mut self, ui: &mut Ui, t: &Theme) {
        let interactive = self.modal.is_none()
            && self.study_menu.is_none()
            && self.compare_menu.is_none()
            && !egui::Popup::is_any_open(ui.ctx());
        let saved = self.draw_styles.clone();
        let styles = move |tool| {
            saved
                .get(&tool)
                .cloned()
                .unwrap_or_else(|| crate::chart::draw::DrawStyle::for_tool(tool))
        };
        let tools = crate::chart::drawing_ui::Tools {
            tool: self.layout.tool,
            magnet: self.layout.magnet,
            lock_all: self.layout.lock_drawings,
            hide_all: self.layout.hide_drawings,
            style: &styles,
            logos: Some(&self.logos),
        };
        let actions = crate::chart::ui::show(ui, &mut self.chart, &mut self.chart_ui, t, interactive, &tools);
        let finished = self.chart_ui.draw.finished;
        if finished && !self.layout.stay_drawing {
            self.layout.tool = Tool::Cross;
        }
        self.chart_actions(actions);
    }
}

/// Shortens `s` to `max` characters with an ellipsis.
fn elide(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(max - 1).collect::<String>())
    }
}

/// A compact text button for the range bar and panel tabs.
pub(crate) fn small_button(ui: &mut Ui, t: &Theme, text: &str, active: bool) -> egui::Response {
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_string(), theme::medium(12.5), t.text);
    let (rect, resp) = ui.allocate_exact_size(vec2(galley.size().x + 16.0, 26.0), Sense::click());
    let hover = motion::toggle(ui.ctx(), resp.id, resp.hovered(), motion::MICRO);
    if ui.is_rect_visible(rect) {
        let p = ui.painter();
        p.rect_filled(rect, CornerRadius::same(6), motion::with_alpha(t.hover_wash(), hover));
        if active {
            p.rect_filled(rect, CornerRadius::same(6), t.accent_soft());
        }
        let color = if active {
            t.accent
        } else {
            motion::lerp_color(t.text2, t.text, hover)
        };
        p.galley_with_override_text_color(rect.center() - galley.size() / 2.0, galley, color);
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// "1 minute", "3 hours", "1 week": TradingView's long interval names.
fn interval_name(tf: Timeframe) -> String {
    let unit = match tf.unit {
        Unit::Second => "second",
        Unit::Minute => "minute",
        Unit::Hour => "hour",
        Unit::Day => "day",
        Unit::Week => "week",
        Unit::Month => "month",
    };
    format!("{} {unit}{}", tf.count, if tf.count == 1 { "" } else { "s" })
}

/// One row of the interval menu. The star sits just right of the name
/// column and shows on hover (always, once starred). Returns (picked,
/// star clicked).
fn interval_row(ui: &mut Ui, t: &Theme, tf: Timeframe, current: bool, fav: bool) -> (bool, bool) {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 28.0), Sense::click());
    let star_rect = Rect::from_center_size(pos2(rect.right() - 16.0, rect.center().y), vec2(24.0, 24.0));
    let star = ui.interact(star_rect, resp.id.with("star"), Sense::click());
    let hover = motion::toggle(ui.ctx(), resp.id, resp.hovered() || star.hovered(), motion::MICRO);
    let p = ui.painter();
    if current {
        p.rect_filled(rect, CornerRadius::same(7), t.accent_soft());
    }
    p.rect_filled(rect, CornerRadius::same(7), motion::with_alpha(t.hover_wash(), hover));
    p.text(
        pos2(rect.left() + 10.0, rect.center().y),
        Align2::LEFT_CENTER,
        interval_name(tf),
        theme::regular(13.5),
        if current { t.accent } else { t.text },
    );
    let star_alpha = if fav { 1.0 } else { hover };
    if star_alpha > 0.0 {
        let c = if fav {
            t.warn
        } else if star.hovered() {
            t.text
        } else {
            t.text3
        };
        if fav {
            w::filled_star(p, star_rect.center(), 6.5, c);
        } else {
            p.text(
                star_rect.center(),
                Align2::CENTER_CENTER,
                ph::STAR,
                theme::regular(14.0),
                motion::with_alpha(c, star_alpha),
            );
        }
    }
    let star = star
        .on_hover_text(if fav {
            "Remove from the top bar"
        } else {
            "Show in the top bar"
        })
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    (resp.clicked(), star.clicked())
}

/// Fills the four corners of `r` outside a rounded rect of radius `rad`
/// with `fill`, so a panel painted square looks rounded.
fn round_corners(p: &egui::Painter, r: Rect, rad: f32, fill: egui::Color32) {
    use std::f32::consts::FRAC_PI_2;
    let corners = [
        (r.left_top(), pos2(r.left() + rad, r.top() + rad), 2.0),
        (r.right_top(), pos2(r.right() - rad, r.top() + rad), 3.0),
        (r.right_bottom(), pos2(r.right() - rad, r.bottom() - rad), 0.0),
        (r.left_bottom(), pos2(r.left() + rad, r.bottom() - rad), 1.0),
    ];
    let steps = 10;
    let mut mesh = egui::Mesh::default();
    for (corner, centre, q) in corners {
        let base = mesh.vertices.len() as u32;
        mesh.colored_vertex(corner, fill);
        for k in 0..=steps {
            let a = (q + k as f32 / steps as f32) * FRAC_PI_2;
            mesh.colored_vertex(centre + vec2(a.cos(), a.sin()) * rad, fill);
        }
        for k in 0..steps as u32 {
            mesh.add_triangle(base, base + 1 + k, base + 2 + k);
        }
    }
    p.add(mesh);
}

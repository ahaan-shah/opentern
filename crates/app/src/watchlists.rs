//! Watchlists, laid out like TradingView's: the list's name (click for its
//! menu, double-click to rename), a circle per list to switch, sections that
//! fold, rows dragged to reorder or move between sections, and live prices.
//! Saved in the workspace; a list is a JSON array of symbols and section
//! headers (older lists, plain symbol arrays, still load).

use crate::app::App;
use crate::icons::ph;
use crate::motion;
use crate::theme::{self, Theme};
use egui::{Align, Align2, CornerRadius, Layout, Rect, Sense, Stroke, Ui, pos2, vec2};
use opentern_core::Symbol;
use opentern_data::Request;
use serde::{Deserialize, Serialize};

/// One row of a list: a section header or a symbol.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Item {
    Section {
        section: String,
        #[serde(default)]
        collapsed: bool,
    },
    Symbol(Symbol),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Watchlist {
    pub id: i64,
    pub name: String,
    pub items: Vec<Item>,
}

impl Watchlist {
    /// Its symbols, in order, without the section headers.
    pub fn symbols(&self) -> Vec<Symbol> {
        self.items
            .iter()
            .filter_map(|i| match i {
                Item::Symbol(s) => Some(s.clone()),
                Item::Section { .. } => None,
            })
            .collect()
    }

    fn sections(&self) -> Vec<(usize, String)> {
        self.items
            .iter()
            .enumerate()
            .filter_map(|(k, i)| match i {
                Item::Section { section, .. } => Some((k, section.clone())),
                Item::Symbol(_) => None,
            })
            .collect()
    }
}

#[derive(Clone, Copy, PartialEq)]
enum SortBy {
    Symbol,
    Last,
    Change,
}

/// What a click in the widget asked for, applied after drawing.
enum Act {
    Pick(Symbol),
    Remove(usize),
    ToTop(usize),
    /// Move item `from` to just after the section header at `to` (or the
    /// top when `None`).
    ToSection(usize, Option<usize>),
    /// Drag: move item `from` to slot `to` (an index among the items).
    Move(usize, usize),
    Fold(usize),
    RenameSection(usize),
    RemoveSection(usize),
}

/// Ids for the bits of edit state the widget keeps between frames.
fn state_id(what: &str) -> egui::Id {
    egui::Id::new(("watchlist", what))
}

impl App {
    pub(crate) fn load_watchlists(store: &opentern_core::Store) -> (Vec<Watchlist>, usize) {
        let mut lists: Vec<Watchlist> = store
            .watchlists()
            .iter()
            .filter_map(|w| {
                Some(Watchlist {
                    id: w.id,
                    name: w.name.clone(),
                    items: serde_json::from_str(&w.json).ok()?,
                })
            })
            .collect();
        if lists.is_empty() {
            lists.push(Watchlist {
                id: 1,
                name: "Watchlist".into(),
                items: opentern_core::starter_watchlist()
                    .into_iter()
                    .map(Item::Symbol)
                    .collect(),
            });
        }
        let active = store
            .value("watchlist_id")
            .and_then(|v| v.parse::<i64>().ok())
            .and_then(|id| lists.iter().position(|l| l.id == id))
            .unwrap_or(0);
        (lists, active)
    }

    pub fn watchlist(&self) -> Vec<Symbol> {
        self.watchlists[self.active_list].symbols()
    }

    pub(crate) fn save_watchlists(&mut self) {
        let rows: Vec<(i64, String, String)> = self
            .watchlists
            .iter()
            .map(|w| {
                (
                    w.id,
                    w.name.clone(),
                    serde_json::to_string(&w.items).unwrap_or_default(),
                )
            })
            .collect();
        if let Err(e) = self.store.save_watchlists(&rows) {
            crate::diag::crumb(format!("couldn't save watchlists: {e}"));
        }
        let id = self.watchlists[self.active_list].id;
        let _ = self.store.set_value("watchlist_id", &id.to_string());
        self.subscribe_quotes();
    }

    /// Live prices for every symbol in the active list.
    pub(crate) fn subscribe_quotes(&mut self) {
        let mut symbols = self.watchlist();
        for s in self.paper_symbols().into_iter().chain(self.indices.symbols()) {
            if !symbols.iter().any(|x| x.id == s.id) {
                symbols.push(s);
            }
        }
        self.hub.send(Request::Quotes { symbols });
    }

    pub fn add_to_watchlist(&mut self, s: Symbol) {
        let list = &mut self.watchlists[self.active_list];
        if !list.items.iter().any(|i| matches!(i, Item::Symbol(x) if x.id == s.id)) {
            list.items.push(Item::Symbol(s));
            self.save_watchlists();
        }
    }

    /// Alt+W: the chart's symbol goes to the end of the open list's top
    /// section (the symbols before the first header, or the first header's
    /// own), from where it can be dragged anywhere.
    pub(crate) fn watch_chart_symbol(&mut self) {
        let s = self.chart.symbol.clone();
        let list = &mut self.watchlists[self.active_list];
        if list.items.iter().any(|i| matches!(i, Item::Symbol(x) if x.id == s.id)) {
            return;
        }
        let start = usize::from(matches!(list.items.first(), Some(Item::Section { .. })));
        let at = list.items[start..]
            .iter()
            .position(|i| matches!(i, Item::Section { .. }))
            .map_or(list.items.len(), |k| start + k);
        list.items.insert(at, Item::Symbol(s));
        self.save_watchlists();
    }

    /// Sorts each section on its own (a second click on a column reverses).
    fn sort_watchlist(&mut self, by: SortBy) {
        let quotes = self.quotes.clone();
        let chg = |s: &Symbol| {
            quotes
                .get(&s.id)
                .filter(|q| q.1 != 0.0)
                .map(|q| q.0 / q.1 - 1.0)
                .unwrap_or(f64::NEG_INFINITY)
        };
        let last = |s: &Symbol| quotes.get(&s.id).map(|q| q.0).unwrap_or(f64::NEG_INFINITY);
        let items = &mut self.watchlists[self.active_list].items;
        let before = items.clone();
        let sort = |run: &mut [Item], reverse: bool| {
            run.sort_by(|a, b| {
                let (Item::Symbol(a), Item::Symbol(b)) = (a, b) else {
                    return std::cmp::Ordering::Equal;
                };
                let o = match by {
                    SortBy::Symbol => a.ticker().cmp(b.ticker()),
                    SortBy::Last => last(b).total_cmp(&last(a)),
                    SortBy::Change => chg(b).total_cmp(&chg(a)),
                };
                if reverse { o.reverse() } else { o }
            });
        };
        for reverse in [false, true] {
            let mut start = 0;
            for k in 0..=items.len() {
                if k == items.len() || matches!(items[k], Item::Section { .. }) {
                    sort(&mut items[start..k], reverse);
                    start = k + 1;
                }
            }
            if *items != before {
                break;
            }
        }
        self.save_watchlists();
    }

    fn new_watchlist(&mut self, ctx: &egui::Context) {
        let id = self.watchlists.iter().map(|w| w.id).max().unwrap_or(0) + 1;
        let name = format!("List {}", self.watchlists.len() + 1);
        self.watchlists.push(Watchlist {
            id,
            name: name.clone(),
            items: Vec::new(),
        });
        self.active_list = self.watchlists.len() - 1;
        self.save_watchlists();
        // Straight into naming it.
        ctx.data_mut(|d| d.insert_temp(state_id("rename"), Some(name)));
    }

    /// The watchlist widget.
    pub(crate) fn watchlist_widget(&mut self, ui: &mut Ui, t: &Theme) {
        self.watchlist_header(ui, t);
        ui.add_space(2.0);
        self.watchlist_switcher(ui, t);
        ui.add_space(6.0);
        self.watchlist_rows(ui, t);
    }

    /// The list's name: a button with its menu (double-click renames), and
    /// the add button.
    fn watchlist_header(&mut self, ui: &mut Ui, t: &Theme) {
        let rename_id = state_id("rename");
        let mut renaming: Option<String> = ui.data(|d| d.get_temp(rename_id)).flatten();
        let mut open_rename = false;
        let mut add_section = false;
        let mut delete = false;
        ui.horizontal(|ui| {
            ui.set_height(30.0);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if crate::widgets::tool_button(ui, t, ph::PLUS, "Add symbol", false, 28.0).clicked() {
                    self.modal = Some(crate::dialogs::Modal::Symbol(crate::dialogs::SymbolSearch::adding()));
                }
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    if let Some(text) = renaming.as_mut() {
                        let r = ui.add(
                            crate::widgets::field(text)
                                .font(theme::semibold(14.0))
                                .desired_width(ui.available_width() - 8.0),
                        );
                        if !r.has_focus() && !r.lost_focus() {
                            r.request_focus();
                        }
                        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                            renaming = None;
                        } else if r.lost_focus() {
                            let name = text.trim().to_string();
                            if !name.is_empty() {
                                self.watchlists[self.active_list].name = name;
                                self.save_watchlists();
                            }
                            renaming = None;
                        }
                        return;
                    }
                    let name = self.watchlists[self.active_list].name.clone();
                    let resp = title_button(ui, t, &name);
                    if resp.double_clicked() {
                        open_rename = true;
                    }
                    self.still_open(ui, &resp, "watchlist");
                    egui::Popup::menu(&resp)
                        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                        .show(|ui| {
                            ui.set_width(210.0);
                            ui.spacing_mut().item_spacing.y = 1.0;
                            let w = crate::widgets::menu_item;
                            if w(ui, t, Some(ph::PENCIL_SIMPLE), "Rename", "", false).clicked() {
                                open_rename = true;
                                ui.close();
                            }
                            if w(ui, t, Some(ph::ROWS), "Add section", "", false).clicked() {
                                add_section = true;
                                ui.close();
                            }
                            if w(ui, t, Some(ph::PLUS), "New list", "", false).clicked() {
                                self.new_watchlist(ui.ctx());
                                ui.close();
                            }
                            if self.watchlists.len() > 1 {
                                crate::widgets::menu_rule(ui, t);
                                if crate::widgets::menu_danger(ui, t, Some(ph::TRASH), "Delete list").clicked() {
                                    delete = true;
                                    ui.close();
                                }
                            }
                        });
                });
            });
        });
        if open_rename {
            renaming = Some(self.watchlists[self.active_list].name.clone());
        }
        ui.data_mut(|d| d.insert_temp(rename_id, renaming));
        if add_section {
            let list = &mut self.watchlists[self.active_list];
            list.items.push(Item::Section {
                section: "New section".into(),
                collapsed: false,
            });
            let at = list.items.len() - 1;
            self.save_watchlists();
            ui.data_mut(|d| d.insert_temp(state_id("section-rename"), Some((at, "New section".to_string()))));
        }
        if delete && self.watchlists.len() > 1 {
            self.watchlists.remove(self.active_list);
            self.active_list = self.active_list.saturating_sub(1);
            self.save_watchlists();
        }
    }

    /// A circle per list with its initial, as on TradingView, and a + for a
    /// new one.
    fn watchlist_switcher(&mut self, ui: &mut Ui, t: &Theme) {
        let mut switch = None;
        let mut new = false;
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
            for (k, l) in self.watchlists.iter().enumerate() {
                let on = k == self.active_list;
                let initial: String = l
                    .name
                    .chars()
                    .find(|c| c.is_alphanumeric())
                    .map(|c| c.to_uppercase().collect())
                    .unwrap_or_else(|| "·".into());
                let tip = format!("{} · {} symbols", l.name, l.symbols().len());
                if circle(ui, t, &initial, on).on_hover_text(tip).clicked() && !on {
                    switch = Some(k);
                }
            }
            if circle(ui, t, ph::PLUS, false).on_hover_text("New list").clicked() {
                new = true;
            }
        });
        if let Some(k) = switch {
            self.active_list = k;
            ui.data_mut(|d| d.insert_temp::<Option<String>>(state_id("rename"), None));
            self.save_watchlists();
        }
        if new {
            self.new_watchlist(ui.ctx());
        }
    }

    fn watchlist_rows(&mut self, ui: &mut Ui, t: &Theme) {
        let list = self.watchlists[self.active_list].clone();
        // Prices first, so the columns can be sized to what they hold.
        let quotes = self.quotes.clone();
        let price = |s: &Symbol| match quotes.get(&s.id) {
            Some((last, prev)) if *prev != 0.0 => {
                let pct = (last / prev - 1.0) * 100.0;
                (
                    crate::format::price(*last, s.precision()),
                    crate::format::signed_pct(pct),
                    if pct >= 0.0 { t.pos } else { t.neg },
                )
            }
            _ => ("—".into(), "—".into(), t.text3),
        };
        let width_of = |text: &str, ui: &Ui| {
            ui.painter()
                .layout_no_wrap(text.to_string(), theme::regular(13.0), t.text)
                .size()
                .x
        };
        let symbols = list.symbols();
        let chg_w = symbols
            .iter()
            .map(|s| width_of(&price(s).1, ui))
            .fold(width_of("Chg%", ui), f32::max);
        let last_w = symbols
            .iter()
            .map(|s| width_of(&price(s).0, ui))
            .fold(width_of("Last", ui), f32::max);
        let w = ui.available_width();
        let (cols, _) = ui.allocate_exact_size(vec2(w, 24.0), Sense::hover());
        let col_last = cols.right() - 4.0 - chg_w - 14.0;
        let name_right = col_last - last_w - 12.0;
        let header = |ui: &mut Ui, x: f32, align: Align2, text: &str, id: &str| -> bool {
            let r = Rect::from_center_size(pos2(x, cols.center().y), vec2(70.0, 22.0));
            let r = match align {
                Align2::LEFT_CENTER => r.translate(vec2(31.0, 0.0)),
                _ => r.translate(vec2(-35.0, 0.0)),
            };
            let resp = ui.interact(r, ui.id().with(id), Sense::click());
            let col = if resp.hovered() { t.text2 } else { t.text3 };
            ui.painter()
                .text(pos2(x, cols.center().y), align, text, theme::medium(11.5), col);
            resp.on_hover_text("Sort").clicked()
        };
        let mut sort = None;
        if header(ui, cols.left() + 4.0, Align2::LEFT_CENTER, "Symbol", "h-sym") {
            sort = Some(SortBy::Symbol);
        }
        if header(ui, col_last, Align2::RIGHT_CENTER, "Last", "h-last") {
            sort = Some(SortBy::Last);
        }
        if header(ui, cols.right() - 4.0, Align2::RIGHT_CENTER, "Chg%", "h-chg") {
            sort = Some(SortBy::Change);
        }
        if let Some(by) = sort {
            self.sort_watchlist(by);
        }

        if list.items.is_empty() {
            crate::widgets::empty_state(ui, t, ph::LIST_PLUS, "An empty list");
        }
        let sections = list.sections();
        let drag_id = state_id("drag");
        let section_edit_id = state_id("section-rename");
        let mut section_edit: Option<(usize, String)> = ui.data(|d| d.get_temp(section_edit_id)).flatten();
        let dragging: Option<usize> = ui.data(|d| d.get_temp(drag_id)).flatten();
        let mut acts: Vec<Act> = Vec::new();
        let mut slots: Vec<(usize, Rect)> = Vec::new();
        let mut hidden = false;
        egui::ScrollArea::vertical().id_salt("watchlist-rows").show(ui, |ui| {
            for (k, item) in list.items.iter().enumerate() {
                match item {
                    Item::Section { section, collapsed } => {
                        hidden = *collapsed;
                        let (rect, resp) =
                            ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::click_and_drag());
                        slots.push((k, rect));
                        if let Some((at, text)) = section_edit.as_mut().filter(|(at, _)| *at == k) {
                            let r = ui.put(
                                rect.shrink2(vec2(2.0, 2.0)),
                                crate::widgets::field(text).font(theme::semibold(11.5)),
                            );
                            if !r.has_focus() && !r.lost_focus() {
                                r.request_focus();
                            }
                            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                                section_edit = None;
                            } else if r.lost_focus() {
                                let at = *at;
                                acts.push(Act::RenameSection(at));
                            }
                            continue;
                        }
                        let inside = ui.rect_contains_pointer(rect);
                        let hov = motion::toggle(ui.ctx(), resp.id, inside, motion::MICRO);
                        let p = ui.painter();
                        p.rect_filled(rect, CornerRadius::same(6), motion::with_alpha(t.hover_wash(), hov));
                        p.text(
                            pos2(rect.left() + 10.0, rect.center().y),
                            Align2::CENTER_CENTER,
                            if *collapsed { ph::CARET_RIGHT } else { ph::CARET_DOWN },
                            theme::regular(11.0),
                            t.text3,
                        );
                        p.text(
                            pos2(rect.left() + 22.0, rect.center().y),
                            Align2::LEFT_CENTER,
                            section.to_uppercase(),
                            theme::semibold(11.5),
                            t.text2,
                        );
                        if resp.clicked() {
                            acts.push(Act::Fold(k));
                        }
                        if resp.double_clicked() {
                            section_edit = Some((k, section.clone()));
                        }
                        if resp.drag_started() {
                            ui.data_mut(|d| d.insert_temp(drag_id, Some(k)));
                        }
                        resp.on_hover_cursor(egui::CursorIcon::PointingHand).context_menu(|ui| {
                            ui.set_width(200.0);
                            ui.spacing_mut().item_spacing.y = 1.0;
                            if crate::widgets::menu_item(ui, t, Some(ph::PENCIL_SIMPLE), "Rename", "", false).clicked()
                            {
                                section_edit = Some((k, section.clone()));
                                ui.close();
                            }
                            if crate::widgets::menu_danger(ui, t, Some(ph::TRASH), "Remove section").clicked() {
                                acts.push(Act::RemoveSection(k));
                                ui.close();
                            }
                        });
                    }
                    Item::Symbol(s) => {
                        if hidden {
                            continue;
                        }
                        let (last, chg, col) = price(s);
                        let active = self.chart.symbol.id == s.id;
                        let (rect, resp) =
                            ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::click_and_drag());
                        slots.push((k, rect));
                        // The pointer, not `hovered()`: the × button on top would
                        // otherwise take the hover, hide itself and flicker.
                        let inside = ui.rect_contains_pointer(rect) && dragging.is_none();
                        let hov = motion::toggle(ui.ctx(), resp.id, inside, motion::MICRO);
                        let lifted = dragging == Some(k);
                        let p = ui.painter();
                        p.rect_filled(rect, CornerRadius::same(6), motion::with_alpha(t.hover_wash(), hov));
                        if active {
                            p.rect_filled(rect, CornerRadius::same(6), t.accent_soft());
                        }
                        if lifted {
                            p.rect_stroke(
                                rect,
                                CornerRadius::same(6),
                                Stroke::new(1.0, t.accent.gamma_multiply(0.6)),
                                egui::StrokeKind::Inside,
                            );
                        }
                        let y = rect.center().y;
                        let name_font = theme::medium(13.0);
                        self.logos.paint(p, t, s, pos2(rect.left() + 15.0, y), 9.0);
                        let room = name_right - rect.left() - 30.0;
                        let name = crate::format::elide_to(p, s.ticker(), &name_font, room);
                        p.text(
                            pos2(rect.left() + 30.0, y),
                            Align2::LEFT_CENTER,
                            name,
                            name_font,
                            if active { t.accent } else { t.text },
                        );
                        p.text(
                            pos2(col_last, y),
                            Align2::RIGHT_CENTER,
                            &last,
                            theme::regular(13.0),
                            t.text,
                        );
                        if inside {
                            let x = Rect::from_center_size(pos2(rect.right() - 12.0, y), vec2(20.0, 20.0));
                            let xr = ui.interact(x, resp.id.with("x"), Sense::click());
                            let c = if xr.hovered() { t.neg } else { t.text3 };
                            ui.painter()
                                .text(x.center(), Align2::CENTER_CENTER, ph::X, theme::regular(12.0), c);
                            if xr.on_hover_text("Remove from watchlist").clicked() {
                                acts.push(Act::Remove(k));
                                continue;
                            }
                        } else {
                            p.text(
                                pos2(rect.right() - 4.0, y),
                                Align2::RIGHT_CENTER,
                                &chg,
                                theme::regular(13.0),
                                col,
                            );
                        }
                        if resp.drag_started() {
                            ui.data_mut(|d| d.insert_temp(drag_id, Some(k)));
                        }
                        let resp = resp
                            .on_hover_text(&s.description)
                            .on_hover_cursor(egui::CursorIcon::PointingHand);
                        if resp.clicked() && !active {
                            acts.push(Act::Pick(s.clone()));
                        }
                        resp.context_menu(|ui| {
                            ui.set_width(220.0);
                            ui.spacing_mut().item_spacing.y = 1.0;
                            let w = crate::widgets::menu_item;
                            if w(ui, t, Some(ph::CHART_LINE), "Open chart", "", false).clicked() {
                                acts.push(Act::Pick(s.clone()));
                                ui.close();
                            }
                            if w(ui, t, Some(ph::ARROW_LINE_UP), "Move to top", "", false).clicked() {
                                acts.push(Act::ToTop(k));
                                ui.close();
                            }
                            if !sections.is_empty() {
                                ui.menu_button(
                                    egui::RichText::new(format!("{}   Move to section", ph::ROWS)).size(13.5),
                                    |ui| {
                                        ui.set_min_width(180.0);
                                        for (at, name) in &sections {
                                            if crate::widgets::menu_item(ui, t, None, name, "", false).clicked() {
                                                acts.push(Act::ToSection(k, Some(*at)));
                                                ui.close();
                                            }
                                        }
                                        if crate::widgets::menu_item(ui, t, None, "No section", "", false).clicked() {
                                            acts.push(Act::ToSection(k, None));
                                            ui.close();
                                        }
                                    },
                                );
                            }
                            if crate::widgets::menu_danger(ui, t, Some(ph::TRASH), "Remove from watchlist").clicked() {
                                acts.push(Act::Remove(k));
                                ui.close();
                            }
                        });
                    }
                }
            }
            // While dragging: a line where the row will land; dropping moves it.
            if let Some(from) = dragging {
                let y = ui.input(|i| i.pointer.interact_pos().map(|p| p.y));
                if let Some(y) = y {
                    let slot = slots
                        .iter()
                        .find(|(_, r)| y < r.center().y)
                        .map(|(k, _)| *k)
                        .unwrap_or(list.items.len());
                    let line_y = slots
                        .iter()
                        .find(|(k, _)| *k == slot)
                        .map(|(_, r)| r.top())
                        .or_else(|| slots.last().map(|(_, r)| r.bottom()));
                    if let Some(ly) = line_y {
                        let r = ui.max_rect();
                        ui.painter()
                            .hline(r.x_range().shrink(4.0), ly, Stroke::new(2.0, t.accent));
                    }
                    if ui.input(|i| i.pointer.any_released()) {
                        acts.push(Act::Move(from, slot));
                        ui.data_mut(|d| d.insert_temp::<Option<usize>>(drag_id, None));
                    }
                } else {
                    ui.data_mut(|d| d.insert_temp::<Option<usize>>(drag_id, None));
                }
            }
        });

        let mut changed = false;
        for a in acts {
            let items = &mut self.watchlists[self.active_list].items;
            match a {
                Act::Pick(s) => self.open_symbol_synced(s),
                Act::Remove(k) => {
                    items.remove(k);
                    changed = true;
                }
                Act::ToTop(k) => {
                    let it = items.remove(k);
                    items.insert(0, it);
                    changed = true;
                }
                Act::ToSection(k, at) => {
                    let it = items.remove(k);
                    let to = match at {
                        Some(at) if at > k => at,
                        Some(at) => at + 1,
                        None => 0,
                    };
                    items.insert(to.min(items.len()), it);
                    changed = true;
                }
                Act::Move(from, to) => {
                    if from < items.len() && to != from && to != from + 1 {
                        let it = items.remove(from);
                        let to = if to > from { to - 1 } else { to };
                        items.insert(to.min(items.len()), it);
                        changed = true;
                    }
                }
                Act::Fold(k) => {
                    if let Some(Item::Section { collapsed, .. }) = items.get_mut(k) {
                        *collapsed = !*collapsed;
                        changed = true;
                    }
                }
                Act::RenameSection(k) => {
                    if let (Some((_, text)), Some(Item::Section { section, .. })) = (&section_edit, items.get_mut(k))
                        && !text.trim().is_empty()
                    {
                        *section = text.trim().to_string();
                        changed = true;
                    }
                    section_edit = None;
                }
                Act::RemoveSection(k) => {
                    items.remove(k);
                    changed = true;
                }
            }
        }
        ui.data_mut(|d| d.insert_temp(section_edit_id, section_edit));
        if changed {
            self.save_watchlists();
        }
    }
}

/// The list's name as a button: a soft wash on hover and while its menu is
/// open, and a caret that turns.
fn title_button(ui: &mut Ui, t: &Theme, name: &str) -> egui::Response {
    let font = theme::semibold(14.0);
    let g = ui.painter().layout_no_wrap(name.to_string(), font, t.text);
    let width = (g.size().x + 40.0).min(ui.available_width());
    let (rect, resp) = ui.allocate_exact_size(vec2(width, 30.0), Sense::click());
    let open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&resp));
    let hover = motion::toggle(ui.ctx(), resp.id, resp.hovered() || open, motion::MICRO);
    let turn = motion::toggle(ui.ctx(), resp.id.with("caret"), open, motion::STANDARD);
    let p = ui.painter();
    p.rect_filled(rect, CornerRadius::same(8), motion::with_alpha(t.hover_wash(), hover));
    let room = rect.width() - 36.0;
    let shown = crate::format::elide_to(p, name, &theme::semibold(14.0), room);
    p.text(
        pos2(rect.left() + 8.0, rect.center().y),
        Align2::LEFT_CENTER,
        shown,
        theme::semibold(14.0),
        t.text,
    );
    let caret = pos2(rect.right() - 14.0, rect.center().y);
    // Turned about its own centre: the galley's top-left goes round it.
    let galley = p.layout_no_wrap(ph::CARET_DOWN.to_string(), theme::regular(11.5), t.text2);
    let angle = turn * std::f32::consts::PI;
    let at = caret - egui::emath::Rot2::from_angle(angle) * (galley.size() / 2.0);
    p.add(egui::epaint::TextShape::new(at, galley, t.text2).with_angle(angle));
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text("List menu (double-click to rename)")
}

/// A round switcher button: the list's initial, filled when it's the one on
/// show.
fn circle(ui: &mut Ui, t: &Theme, text: &str, on: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(26.0, 26.0), Sense::click());
    let hover = motion::toggle(ui.ctx(), resp.id, resp.hovered(), motion::MICRO);
    let on_k = motion::toggle(ui.ctx(), resp.id.with("on"), on, motion::STANDARD);
    let p = ui.painter();
    let c = rect.center();
    p.circle_filled(c, 12.5, motion::with_alpha(t.text, on_k));
    p.circle_filled(c, 12.5, motion::with_alpha(t.hover_wash(), hover * (1.0 - on_k)));
    p.circle_stroke(c, 12.0, Stroke::new(1.0, motion::with_alpha(t.border, 1.0 - on_k)));
    p.text(
        c,
        Align2::CENTER_CENTER,
        text,
        theme::medium(12.0),
        motion::lerp_color(t.text2, t.bg, on_k),
    );
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Lists saved before sections (plain symbol arrays) still load, and
    /// sections round-trip.
    #[test]
    fn old_lists_load_and_sections_round_trip() {
        let old = serde_json::to_string(&opentern_core::starter_watchlist()[..2]).unwrap();
        let items: Vec<Item> = serde_json::from_str(&old).unwrap();
        assert_eq!(items.len(), 2);
        assert!(items.iter().all(|i| matches!(i, Item::Symbol(_))));
        let mut with = items.clone();
        with.insert(
            1,
            Item::Section {
                section: "Majors".into(),
                collapsed: true,
            },
        );
        let back: Vec<Item> = serde_json::from_str(&serde_json::to_string(&with).unwrap()).unwrap();
        assert_eq!(back, with);
        let w = Watchlist {
            id: 1,
            name: "x".into(),
            items: back,
        };
        assert_eq!(w.symbols().len(), 2);
        assert_eq!(w.sections(), vec![(1, "Majors".to_string())]);
    }
}

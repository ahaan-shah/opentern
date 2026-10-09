//! Watchlists: several named lists of symbols, each editable (add, remove,
//! reorder, sort), with live prices. Saved in the workspace.

use crate::app::App;
use crate::icons::ph;
use crate::motion;
use crate::theme::{self, Theme};
use egui::{Align, Align2, Color32, CornerRadius, Layout, Sense, Ui, pos2, vec2};
use opentern_core::Symbol;
use opentern_data::Request;

#[derive(Clone, Debug, PartialEq)]
pub struct Watchlist {
    pub id: i64,
    pub name: String,
    pub symbols: Vec<Symbol>,
}

#[derive(Clone, Copy, PartialEq)]
enum SortBy {
    Symbol,
    Last,
    Change,
}

/// Text being typed into the list menu (new list / rename).
#[derive(Clone, Default)]
pub struct ListEdit {
    pub new_name: String,
    pub rename: String,
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
                    symbols: serde_json::from_str(&w.json).ok()?,
                })
            })
            .collect();
        if lists.is_empty() {
            lists.push(Watchlist {
                id: 1,
                name: "Watchlist".into(),
                symbols: opentern_core::starter_watchlist(),
            });
        }
        let active = store
            .value("watchlist_id")
            .and_then(|v| v.parse::<i64>().ok())
            .and_then(|id| lists.iter().position(|l| l.id == id))
            .unwrap_or(0);
        (lists, active)
    }

    pub fn watchlist(&self) -> &[Symbol] {
        &self.watchlists[self.active_list].symbols
    }

    pub(crate) fn save_watchlists(&mut self) {
        let rows: Vec<(i64, String, String)> = self
            .watchlists
            .iter()
            .map(|w| {
                (
                    w.id,
                    w.name.clone(),
                    serde_json::to_string(&w.symbols).unwrap_or_default(),
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
        self.hub.send(Request::Quotes {
            symbols: self.watchlist().to_vec(),
        });
    }

    pub fn add_to_watchlist(&mut self, s: Symbol) {
        let list = &mut self.watchlists[self.active_list];
        if !list.symbols.iter().any(|x| x.id == s.id) {
            list.symbols.push(s);
            self.save_watchlists();
        }
    }

    fn remove_from_watchlist(&mut self, id: &str) {
        self.watchlists[self.active_list].symbols.retain(|s| s.id != id);
        self.save_watchlists();
    }

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
        let list = &mut self.watchlists[self.active_list].symbols;
        let before = list.clone();
        match by {
            SortBy::Symbol => list.sort_by(|a, b| a.ticker().cmp(b.ticker())),
            SortBy::Last => list.sort_by(|a, b| last(b).total_cmp(&last(a))),
            SortBy::Change => list.sort_by(|a, b| chg(b).total_cmp(&chg(a))),
        }
        // A second click on the same column reverses.
        if *list == before {
            list.reverse();
        }
        self.save_watchlists();
    }

    /// The watchlist widget: list menu, add button, sortable columns, rows
    /// with live prices; hover a row for its remove button, right-click for
    /// more.
    pub(crate) fn watchlist_widget(&mut self, ui: &mut Ui, t: &Theme) {
        // Header: list name with its menu, and the add button.
        ui.horizontal(|ui| {
            let name = self.watchlists[self.active_list].name.clone();
            let resp = ui.add(
                egui::Button::new(
                    egui::RichText::new(format!("{name}  {}", ph::CARET_DOWN)).font(theme::semibold(14.0)),
                )
                .frame(false),
            );
            egui::Popup::menu(&resp)
                .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                .show(|ui| self.list_menu(ui, t));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if crate::widgets::tool_button(ui, t, ph::PLUS, "Add symbol", false, 26.0).clicked() {
                    self.modal = Some(crate::dialogs::Modal::Symbol(crate::dialogs::SymbolSearch::adding()));
                }
            });
        });
        ui.add_space(4.0);
        let w = ui.available_width();
        let (cols, _) = ui.allocate_exact_size(vec2(w, 24.0), Sense::hover());
        let col_last = cols.right() - w * 0.30;
        let header = |ui: &mut Ui, x: f32, align: Align2, text: &str, id: &str| -> bool {
            let r = egui::Rect::from_center_size(pos2(x, cols.center().y), vec2(70.0, 22.0));
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
        let mut pick = None;
        let mut remove = None;
        let mut to_top = None;
        egui::ScrollArea::vertical().id_salt("watchlist-rows").show(ui, |ui| {
            for s in self.watchlist().to_vec() {
                let active = self.chart.symbol.id == s.id;
                let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::click());
                let hov = motion::toggle(ui.ctx(), resp.id, resp.hovered(), motion::MICRO);
                let p = ui.painter();
                p.rect_filled(rect, CornerRadius::same(6), motion::with_alpha(t.hover_wash(), hov));
                if active {
                    p.rect_filled(rect, CornerRadius::same(6), t.accent_soft());
                }
                let y = rect.center().y;
                let (last, chg, col) = match self.quotes.get(&s.id) {
                    Some((last, prev)) if *prev != 0.0 => {
                        let pct = (last / prev - 1.0) * 100.0;
                        let col = if pct >= 0.0 { t.pos } else { t.neg };
                        (
                            crate::format::price(*last, s.precision()),
                            crate::format::signed_pct(pct),
                            col,
                        )
                    }
                    _ => ("—".into(), "—".into(), t.text3),
                };
                let name_font = theme::medium(13.0);
                // The name gets what's left of the row, clipped so it never
                // runs into the prices.
                let name_clip = egui::Rect::from_min_max(rect.min, pos2(col_last - 80.0, rect.max.y));
                p.with_clip_rect(name_clip).text(
                    pos2(rect.left() + 6.0, y),
                    Align2::LEFT_CENTER,
                    s.ticker(),
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
                if resp.hovered() {
                    let x = egui::Rect::from_center_size(pos2(rect.right() - 12.0, y), vec2(20.0, 20.0));
                    let xr = ui.interact(x, resp.id.with("x"), Sense::click());
                    let c = if xr.hovered() { t.neg } else { t.text3 };
                    ui.painter()
                        .text(x.center(), Align2::CENTER_CENTER, ph::X, theme::regular(12.0), c);
                    if xr.on_hover_text("Remove from watchlist").clicked() {
                        remove = Some(s.id.clone());
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
                let resp = resp
                    .on_hover_text(&s.description)
                    .on_hover_cursor(egui::CursorIcon::PointingHand);
                if resp.clicked() && !active {
                    pick = Some(s.clone());
                }
                resp.context_menu(|ui| {
                    if ui.button("Open chart").clicked() {
                        pick = Some(s.clone());
                        ui.close();
                    }
                    if ui.button("Move to top").clicked() {
                        to_top = Some(s.id.clone());
                        ui.close();
                    }
                    if ui
                        .button(egui::RichText::new("Remove from watchlist").color(t.neg))
                        .clicked()
                    {
                        remove = Some(s.id.clone());
                        ui.close();
                    }
                });
            }
        });
        if let Some(id) = remove {
            self.remove_from_watchlist(&id);
        }
        if let Some(id) = to_top {
            let list = &mut self.watchlists[self.active_list].symbols;
            if let Some(i) = list.iter().position(|s| s.id == id) {
                let s = list.remove(i);
                list.insert(0, s);
            }
            self.save_watchlists();
        }
        if let Some(s) = pick {
            self.open_symbol(s);
        }
    }

    fn list_menu(&mut self, ui: &mut Ui, t: &Theme) {
        ui.set_min_width(230.0);
        let mut switch = None;
        for (i, l) in self.watchlists.iter().enumerate() {
            let on = i == self.active_list;
            let text = egui::RichText::new(format!("{}  ({})", l.name, l.symbols.len())).color(if on {
                t.accent
            } else {
                t.text
            });
            if ui.add(egui::Button::new(text).frame(false)).clicked() {
                switch = Some(i);
            }
        }
        if let Some(i) = switch {
            self.active_list = i;
            self.save_watchlists();
            ui.close();
        }
        ui.separator();
        let mut edit: crate::watchlists::ListEdit =
            ui.data(|d| d.get_temp(ui.id().with("list-edit"))).unwrap_or_default();
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut edit.new_name)
                    .hint_text("New list name")
                    .desired_width(150.0),
            );
            if ui
                .add_enabled(!edit.new_name.trim().is_empty(), egui::Button::new("Create"))
                .clicked()
            {
                let id = self.watchlists.iter().map(|w| w.id).max().unwrap_or(0) + 1;
                self.watchlists.push(Watchlist {
                    id,
                    name: edit.new_name.trim().to_string(),
                    symbols: Vec::new(),
                });
                self.active_list = self.watchlists.len() - 1;
                edit.new_name.clear();
                self.save_watchlists();
            }
        });
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut edit.rename)
                    .hint_text("Rename this list")
                    .desired_width(150.0),
            );
            if ui
                .add_enabled(!edit.rename.trim().is_empty(), egui::Button::new("Rename"))
                .clicked()
            {
                self.watchlists[self.active_list].name = edit.rename.trim().to_string();
                edit.rename.clear();
                self.save_watchlists();
            }
        });
        ui.data_mut(|d| d.insert_temp(ui.id().with("list-edit"), edit));
        if self.watchlists.len() > 1
            && ui
                .button(egui::RichText::new("Delete this list").color(t.neg))
                .clicked()
        {
            self.watchlists.remove(self.active_list);
            self.active_list = 0;
            self.save_watchlists();
            ui.close();
        }
        let _ = Color32::TRANSPARENT;
    }
}

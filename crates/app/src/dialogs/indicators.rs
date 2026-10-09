//! The Indicators dialog: search, favourites and categories on the left,
//! the list on the right. Click adds to the chart (the dialog stays open,
//! so several can be added); the star marks a favourite.

use super::{Done, frame};
use crate::app::App;
use crate::icons::ph;
use crate::motion;
use crate::theme::{self, Theme};
use egui::{Align2, CornerRadius, Sense, Ui, pos2, vec2};
use opentern_series::builtins;
use opentern_series::study::{Category, Meta};

#[derive(Clone, Copy, PartialEq)]
enum Tab {
    Favorites,
    All,
    Cat(Category),
}

pub struct Indicators {
    query: String,
    tab: Tab,
    focused: bool,
}

impl Indicators {
    pub fn new(has_favorites: bool) -> Indicators {
        Indicators {
            query: String::new(),
            tab: if has_favorites { Tab::Favorites } else { Tab::All },
            focused: false,
        }
    }

    pub fn show(&mut self, ui: &mut Ui, app: &mut App, t: &Theme) -> Done {
        let mut done = Done::Keep;
        let mut add: Option<&'static Meta> = None;
        let mut star: Option<&'static str> = None;
        if frame(ui, t, "Indicators", 720.0, |ui| {
            let r = ui.add(
                egui::TextEdit::singleline(&mut self.query)
                    .hint_text(format!("{}  Search", ph::MAGNIFYING_GLASS))
                    .desired_width(f32::INFINITY)
                    .margin(vec2(10.0, 7.0)),
            );
            if !self.focused {
                r.request_focus();
                self.focused = true;
            }
            ui.add_space(8.0);
            let height = 420.0;
            ui.horizontal_top(|ui| {
                // Left column: where to look.
                ui.allocate_ui_with_layout(vec2(170.0, height), egui::Layout::top_down(egui::Align::Min), |ui| {
                    ui.set_min_size(vec2(170.0, height));
                    let mut item = |ui: &mut Ui, tab: Tab, label: &str| {
                        let on = self.tab == tab && self.query.is_empty();
                        let (rect, resp) = ui.allocate_exact_size(vec2(170.0, 30.0), Sense::click());
                        let hov = motion::toggle(ui.ctx(), resp.id, resp.hovered(), motion::MICRO);
                        let bg = if on {
                            t.accent_soft()
                        } else {
                            motion::with_alpha(t.hover_wash(), hov)
                        };
                        ui.painter().rect_filled(rect, CornerRadius::same(7), bg);
                        ui.painter().text(
                            pos2(rect.left() + 10.0, rect.center().y),
                            Align2::LEFT_CENTER,
                            label,
                            theme::medium(13.0),
                            if on { t.accent } else { t.text },
                        );
                        if resp.clicked() {
                            self.tab = tab;
                            self.query.clear();
                        }
                    };
                    item(ui, Tab::Favorites, &format!("{}  Favorites", ph::STAR));
                    item(ui, Tab::All, &format!("{}  Built-ins", ph::FUNCTION));
                    ui.add_space(4.0);
                    for c in Category::ALL {
                        item(ui, Tab::Cat(c), &format!("      {}", c.name()));
                    }
                });
                let (line, _) = ui.allocate_exact_size(vec2(17.0, height), Sense::hover());
                ui.painter()
                    .vline(line.center().x, line.y_range(), egui::Stroke::new(1.0, t.border));
                // Right: the list.
                let w = ui.available_width();
                ui.allocate_ui_with_layout(vec2(w, height), egui::Layout::top_down(egui::Align::Min), |ui| {
                    let q = self.query.trim().to_lowercase();
                    let list: Vec<&'static Meta> = builtins::all()
                        .iter()
                        .filter(|m| {
                            if !q.is_empty() {
                                return m.name.to_lowercase().contains(&q) || m.short.to_lowercase().contains(&q);
                            }
                            match self.tab {
                                Tab::Favorites => app.store.is_favorite("indicator", m.id),
                                Tab::All => true,
                                Tab::Cat(c) => m.category == c,
                            }
                        })
                        .collect();
                    egui::ScrollArea::vertical()
                        .id_salt("indicator-list")
                        .max_height(height)
                        .show(ui, |ui| {
                            ui.set_width(w - 12.0);
                            if list.is_empty() {
                                ui.add_space(10.0);
                                let msg = if self.tab == Tab::Favorites && q.is_empty() {
                                    "Star an indicator to keep it here."
                                } else {
                                    "Nothing matches."
                                };
                                ui.label(egui::RichText::new(msg).color(t.text3));
                            }
                            for m in list {
                                let (rect, resp) =
                                    ui.allocate_exact_size(vec2(ui.available_width(), 32.0), Sense::click());
                                let hov = motion::toggle(ui.ctx(), resp.id, resp.hovered(), motion::MICRO);
                                ui.painter().rect_filled(
                                    rect,
                                    CornerRadius::same(7),
                                    motion::with_alpha(t.hover_wash(), hov),
                                );
                                let fav = app.store.is_favorite("indicator", m.id);
                                let star_rect = egui::Rect::from_center_size(
                                    pos2(rect.left() + 16.0, rect.center().y),
                                    vec2(22.0, 22.0),
                                );
                                let star_resp = ui.interact(star_rect, resp.id.with("star"), Sense::click());
                                let star_col = if fav {
                                    t.warn
                                } else if star_resp.hovered() {
                                    t.text2
                                } else {
                                    motion::with_alpha(t.text3, 0.3 + hov * 0.7)
                                };
                                let glyph = ph::STAR;
                                ui.painter().text(
                                    star_rect.center(),
                                    Align2::CENTER_CENTER,
                                    glyph,
                                    theme::regular(14.0),
                                    star_col,
                                );
                                ui.painter().text(
                                    pos2(rect.left() + 36.0, rect.center().y),
                                    Align2::LEFT_CENTER,
                                    m.name,
                                    theme::regular(13.5),
                                    t.text,
                                );
                                ui.painter().text(
                                    pos2(rect.right() - 10.0, rect.center().y),
                                    Align2::RIGHT_CENTER,
                                    m.category.name(),
                                    theme::regular(12.0),
                                    t.text3,
                                );
                                if star_resp
                                    .on_hover_text(if fav {
                                        "Remove from favorites"
                                    } else {
                                        "Add to favorites"
                                    })
                                    .clicked()
                                {
                                    star = Some(m.id);
                                } else if resp.clicked() {
                                    add = Some(m);
                                }
                            }
                        });
                });
            });
        }) {
            done = Done::Close;
        }
        if let Some(id) = star {
            app.toggle_favorite("indicator", id);
        }
        if let Some(m) = add {
            app.add_study(m);
        }
        done
    }
}

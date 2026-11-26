//! Floating Favorite Drawing Tools Toolbar over the chart.
//!
//! Provides quick 1-click access to favorited drawing tools, matching
//! TradingView's favorite drawings bar: draggable handle, active tool
//! highlight, horizontal/vertical orientation toggle, collapse/expand,
//! close/hide, drag-and-drop reordering, and context menu.

use crate::app::App;
use crate::chart::draw::Tool;
use crate::i18n::tr;
use crate::icons::ph;
use crate::motion;
use crate::theme::{self, Theme};
use egui::{Align2, CornerRadius, CursorIcon, Id, Margin, Pos2, Sense, Stroke, Ui, vec2};

const BTN_SIZE: f32 = 28.0;
const ICON_SIZE: f32 = 15.0;

impl App {
    /// Renders the floating favorite drawing tools toolbar over the chart.
    pub(crate) fn favorite_toolbar(&mut self, ctx: &egui::Context) {
        if !self.layout.fav_toolbar {
            return;
        }
        if self.settings.is_some() || self.overview_open() || self.modal.is_some() {
            return;
        }
        let chart_rect = self.chart_ui.main_rect;
        if !chart_rect.is_positive() {
            return;
        }

        let t = self.t();
        let tools = self.favorite_tools();
        let vertical = self.layout.fav_toolbar_vertical;
        let collapsed = self.layout.fav_toolbar_collapsed;

        // Estimated dimensions for initial placement.
        let num_items = tools.len().max(1);
        let est_w = if collapsed {
            74.0
        } else if vertical {
            40.0
        } else {
            // margin(12) + grip(15) + divider(4) + 3 utility buttons(75) + tools * 31
            106.0 + (num_items as f32) * 31.0
        };
        let est_h = if collapsed {
            36.0
        } else if vertical {
            106.0 + (num_items as f32) * 31.0
        } else {
            36.0
        };

        // Clamp to chart bounds so toolbar stays within the active chart.
        let min_x = chart_rect.left() + 6.0;
        let max_x = (chart_rect.right() - est_w - 6.0).max(min_x);
        let min_y = chart_rect.top() + 6.0;
        let max_y = (chart_rect.bottom() - est_h - 6.0).max(min_y);

        // Default position: centered horizontally near the bottom for horizontal, left-center for vertical.
        let default_pos = if vertical {
            Pos2::new(min_x + 10.0, (chart_rect.center().y - est_h / 2.0).clamp(min_y, max_y))
        } else {
            Pos2::new(
                (chart_rect.center().x - est_w / 2.0).clamp(min_x, max_x),
                (chart_rect.bottom() - est_h - 60.0).clamp(min_y, max_y),
            )
        };

        let mut current_pos = self
            .layout
            .fav_toolbar_pos
            .map(|[x, y]| Pos2::new(x, y))
            .unwrap_or(default_pos);

        current_pos.x = current_pos.x.clamp(min_x, max_x);
        current_pos.y = current_pos.y.clamp(min_y, max_y);

        let mut pos_changed = false;
        let mut close_toolbar = false;
        let mut toggle_orientation = false;
        let mut toggle_collapse = false;
        let mut pick_tool: Option<Tool> = None;
        let mut remove_tool: Option<Tool> = None;
        let mut move_act: Option<(usize, usize)> = None;

        egui::Area::new(Id::new("fav-draw-toolbar"))
            .order(egui::Order::Foreground)
            .fixed_pos(current_pos)
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(t.elevated)
                    .stroke(Stroke::new(1.0, t.border))
                    .corner_radius(CornerRadius::same(8))
                    .inner_margin(Margin::symmetric(6, 4))
                    .shadow(egui::Shadow {
                        offset: [0, 4],
                        blur: 16,
                        spread: 0,
                        color: t.shadow(),
                    })
                    .show(ui, |ui| {
                        let mut render_body = |ui: &mut Ui| {
                            // 1. Drag grip handle.
                            let grip_glyph = if vertical { ph::DOTS_SIX } else { ph::DOTS_SIX_VERTICAL };
                            let grip_size = if vertical {
                                vec2(BTN_SIZE, 12.0)
                            } else {
                                vec2(12.0, BTN_SIZE)
                            };
                            let (grip_rect, grip_resp) = ui.allocate_exact_size(grip_size, Sense::drag());
                            let grip_hover = motion::hover(&grip_resp);
                            let grip_col = motion::lerp_color(t.text3, t.text, grip_hover);
                            ui.painter().text(
                                grip_rect.center(),
                                Align2::CENTER_CENTER,
                                grip_glyph,
                                theme::regular(14.0),
                                grip_col,
                            );
                            let is_dragged = grip_resp.dragged();
                            let delta = grip_resp.drag_delta();
                            let _grip_resp = grip_resp
                                .on_hover_cursor(if is_dragged {
                                    CursorIcon::Grabbing
                                } else {
                                    CursorIcon::Grab
                                })
                                .on_hover_text(tr("Drag to move"));
                            if is_dragged {
                                current_pos += delta;
                                pos_changed = true;
                            }

                            // 2. Collapsed state: grip + star + expand + close.
                            if collapsed {
                                ui.add_space(2.0);
                                let star_rect = ui.allocate_exact_size(vec2(16.0, BTN_SIZE), Sense::hover()).0;
                                crate::widgets::filled_star(ui.painter(), star_rect.center(), 6.0, t.warn);
                                let expand_glyph = if vertical { ph::CARET_DOWN } else { ph::CARET_RIGHT };
                                if tool_btn(ui, &t, expand_glyph, tr("Expand toolbar"), false).clicked() {
                                    toggle_collapse = true;
                                }
                                if tool_btn(ui, &t, ph::X, tr("Hide toolbar"), false).clicked() {
                                    close_toolbar = true;
                                }
                                return;
                            }

                            // 3. Empty state: when no tools are favorited.
                            if tools.is_empty() {
                                ui.add_space(4.0);
                                ui.label(
                                    egui::RichText::new(tr("No favorites yet"))
                                        .font(theme::regular(12.0))
                                        .color(t.text3),
                                );
                                ui.add_space(4.0);
                                if tool_btn(ui, &t, ph::X, tr("Hide toolbar"), false).clicked() {
                                    close_toolbar = true;
                                }
                                return;
                            }

                            // 4. Tool buttons.
                            let drag_id = Id::new("fav-toolbar-tool-drag");
                            let dragging_from: Option<usize> = ui.data(|d| d.get_temp(drag_id));

                            for (idx, &tool) in tools.iter().enumerate() {
                                let active = self.layout.tool == tool;
                                let tip = format!("{} ({})", tr(tool.name()), tool.shortcut());
                                let (rect, resp) =
                                    ui.allocate_exact_size(vec2(BTN_SIZE, BTN_SIZE), Sense::click_and_drag());
                                let hover = motion::hover(&resp);

                                // Background fill.
                                if active {
                                    ui.painter().rect_filled(rect, CornerRadius::same(6), t.accent_soft());
                                } else if hover > 0.0 {
                                    ui.painter().rect_filled(
                                        rect,
                                        CornerRadius::same(6),
                                        motion::with_alpha(t.hover_wash(), hover),
                                    );
                                }

                                // Tool icon.
                                let col = if active {
                                    t.accent
                                } else {
                                    motion::lerp_color(t.text2, t.text, hover)
                                };
                                ui.painter().text(
                                    rect.center(),
                                    Align2::CENTER_CENTER,
                                    tool.icon(),
                                    theme::regular(ICON_SIZE),
                                    col,
                                );

                                // Reordering drag indicator.
                                if let Some(from) = dragging_from
                                    && resp.hovered()
                                    && from != idx
                                {
                                    let stroke = Stroke::new(2.0, t.accent);
                                    if vertical {
                                        let y = if from < idx { rect.bottom() } else { rect.top() };
                                        ui.painter().hline(rect.x_range(), y, stroke);
                                    } else {
                                        let x = if from < idx { rect.right() } else { rect.left() };
                                        ui.painter().vline(x, rect.y_range(), stroke);
                                    }
                                }

                                if resp.drag_started() {
                                    ui.data_mut(|d| d.insert_temp(drag_id, Some(idx)));
                                }

                                if resp.clicked() {
                                    pick_tool = Some(if active { Tool::Cross } else { tool });
                                }

                                // Context menu on right click.
                                egui::Popup::context_menu(&resp).show(|ui| {
                                    ui.set_width(170.0);
                                    ui.spacing_mut().item_spacing.y = 1.0;
                                    if crate::widgets::menu_item(
                                        ui,
                                        &t,
                                        Some(ph::STAR),
                                        tr("Remove from favorites"),
                                        "",
                                        false,
                                    )
                                    .clicked()
                                    {
                                        remove_tool = Some(tool);
                                        ui.close();
                                    }
                                    if idx > 0 {
                                        let move_prev_tip = if vertical { tr("Move up") } else { tr("Move left") };
                                        let move_prev_icon = if vertical { ph::ARROW_UP } else { ph::ARROW_LEFT };
                                        if crate::widgets::menu_item(
                                            ui,
                                            &t,
                                            Some(move_prev_icon),
                                            move_prev_tip,
                                            "",
                                            false,
                                        )
                                        .clicked()
                                        {
                                            move_act = Some((idx, idx - 1));
                                            ui.close();
                                        }
                                    }
                                    if idx + 1 < tools.len() {
                                        let move_next_tip = if vertical { tr("Move down") } else { tr("Move right") };
                                        let move_next_icon = if vertical { ph::ARROW_DOWN } else { ph::ARROW_RIGHT };
                                        if crate::widgets::menu_item(
                                            ui,
                                            &t,
                                            Some(move_next_icon),
                                            move_next_tip,
                                            "",
                                            false,
                                        )
                                        .clicked()
                                        {
                                            move_act = Some((idx, idx + 1));
                                            ui.close();
                                        }
                                    }
                                });

                                resp.on_hover_cursor(CursorIcon::PointingHand).on_hover_text(tip);
                            }

                            // Handle dropping a reordered tool.
                            if let Some(from) = dragging_from
                                && ui.input(|i| i.pointer.any_released())
                            {
                                if let Some(hover_pos) = ui.ctx().pointer_hover_pos() {
                                    let target = tools.iter().enumerate().min_by_key(|(_, t)| {
                                        let est_tool_center = if vertical {
                                            current_pos.y + 20.0 + (idx_to_f32(t, &tools)) * 31.0
                                        } else {
                                            current_pos.x + 20.0 + (idx_to_f32(t, &tools)) * 31.0
                                        };
                                        let pointer_val = if vertical { hover_pos.y } else { hover_pos.x };
                                        ((est_tool_center - pointer_val).abs() * 10.0) as i64
                                    });
                                    if let Some((to, _)) = target
                                        && from != to
                                    {
                                        move_act = Some((from, to));
                                    }
                                }
                                ui.data_mut(|d| d.insert_temp::<Option<usize>>(drag_id, None));
                            }

                            // 5. Divider.
                            let div_size = if vertical {
                                vec2(BTN_SIZE, 1.0)
                            } else {
                                vec2(1.0, BTN_SIZE)
                            };
                            let div_rect = ui.allocate_exact_size(div_size, Sense::hover()).0;
                            ui.painter().rect_filled(div_rect, CornerRadius::ZERO, t.border);

                            // 6. Orientation toggle.
                            let orient_glyph = if vertical { ph::ROWS } else { ph::COLUMNS };
                            let orient_tip = if vertical {
                                tr("Horizontal toolbar")
                            } else {
                                tr("Vertical toolbar")
                            };
                            if tool_btn(ui, &t, orient_glyph, orient_tip, false).clicked() {
                                toggle_orientation = true;
                            }

                            // 7. Collapse toggle.
                            let collapse_glyph = if vertical { ph::CARET_UP } else { ph::CARET_LEFT };
                            if tool_btn(ui, &t, collapse_glyph, tr("Collapse toolbar"), false).clicked() {
                                toggle_collapse = true;
                            }

                            // 8. Close button.
                            if tool_btn(ui, &t, ph::X, tr("Hide toolbar"), false).clicked() {
                                close_toolbar = true;
                            }
                        };

                        if vertical {
                            ui.vertical(|ui| {
                                ui.spacing_mut().item_spacing = vec2(0.0, 3.0);
                                render_body(ui);
                            });
                        } else {
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing = vec2(3.0, 0.0);
                                render_body(ui);
                            });
                        }
                    });
            });

        // Apply state changes.
        if pos_changed {
            self.layout.fav_toolbar_pos = Some([current_pos.x, current_pos.y]);
            self.save_fav_toolbar_settings();
        }
        if close_toolbar {
            self.layout.fav_toolbar = false;
            self.save_fav_toolbar_settings();
        }
        if toggle_orientation {
            self.layout.fav_toolbar_vertical = !self.layout.fav_toolbar_vertical;
            self.save_fav_toolbar_settings();
        }
        if toggle_collapse {
            self.layout.fav_toolbar_collapsed = !self.layout.fav_toolbar_collapsed;
            self.save_fav_toolbar_settings();
        }
        if let Some(tool) = pick_tool {
            self.pick_tool(tool);
        }
        if let Some(tool) = remove_tool {
            self.toggle_favorite_tool(tool);
        }
        if let Some((from, to)) = move_act {
            self.move_favorite_tool(from, to);
        }
    }
}

fn idx_to_f32(target: &Tool, list: &[Tool]) -> f32 {
    list.iter().position(|t| t == target).unwrap_or(0) as f32
}

/// A compact icon button for toolbar utilities (close, orientation, collapse).
fn tool_btn(ui: &mut Ui, t: &Theme, glyph: &str, tip: &str, active: bool) -> egui::Response {
    let size = 22.0;
    let (rect, resp) = ui.allocate_exact_size(vec2(size, size), Sense::click());
    let hover = motion::hover(&resp);
    if active {
        ui.painter().rect_filled(rect, CornerRadius::same(5), t.accent_soft());
    } else if hover > 0.0 {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(5), motion::with_alpha(t.hover_wash(), hover));
    }
    let col = if active {
        t.accent
    } else {
        motion::lerp_color(t.text3, t.text, hover)
    };
    ui.painter()
        .text(rect.center(), Align2::CENTER_CENTER, glyph, theme::regular(13.0), col);
    resp.on_hover_cursor(CursorIcon::PointingHand).on_hover_text(tip)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chart::draw::DEFAULT_FAVORITE_TOOLS;
    use crate::drive::Driver;
    use crate::multi::Grid;
    use egui::{Key, Modifiers};
    use opentern_core::Store;

    #[test]
    fn test_default_favorite_tools_initialization() {
        let d = Driver::new("fav-defaults");
        let favs = d.app.favorite_tools();
        assert_eq!(favs.len(), 12, "initial favorites count");
        assert_eq!(favs, DEFAULT_FAVORITE_TOOLS);
        assert!(d.app.is_tool_favorite(Tool::Cross));
        assert!(d.app.is_tool_favorite(Tool::TrendLine));
        assert!(d.app.is_tool_favorite(Tool::Text));
        assert!(!d.app.is_tool_favorite(Tool::Pitchfork));
    }

    #[test]
    fn test_add_remove_toggle_favorite() {
        let mut d = Driver::new("fav-toggle");
        assert!(!d.app.is_tool_favorite(Tool::Pitchfork));

        // Add pitchfork.
        d.app.toggle_favorite_tool(Tool::Pitchfork);
        assert!(d.app.is_tool_favorite(Tool::Pitchfork));
        assert!(d.app.favorite_tools().contains(&Tool::Pitchfork));
        assert!(d.app.layout.fav_toolbar, "adding favorite reveals toolbar");

        // Remove pitchfork.
        d.app.toggle_favorite_tool(Tool::Pitchfork);
        assert!(!d.app.is_tool_favorite(Tool::Pitchfork));
        assert!(!d.app.favorite_tools().contains(&Tool::Pitchfork));
    }

    #[test]
    fn test_duplicate_prevention() {
        let mut d = Driver::new("fav-dups");
        let tools_with_dups = vec![
            Tool::TrendLine,
            Tool::TrendLine,
            Tool::HorizontalLine,
            Tool::TrendLine,
            Tool::Rectangle,
            Tool::Rectangle,
        ];
        d.app.set_favorite_tools(&tools_with_dups);
        let favs = d.app.favorite_tools();
        assert_eq!(
            favs,
            vec![Tool::TrendLine, Tool::HorizontalLine, Tool::Rectangle],
            "duplicates filtered out"
        );
    }

    #[test]
    fn test_reordering() {
        let mut d = Driver::new("fav-reorder");
        d.app
            .set_favorite_tools(&[Tool::Cross, Tool::TrendLine, Tool::Rectangle]);
        assert_eq!(
            d.app.favorite_tools(),
            vec![Tool::Cross, Tool::TrendLine, Tool::Rectangle]
        );

        // Move Rectangle from index 2 to index 0.
        d.app.move_favorite_tool(2, 0);
        assert_eq!(
            d.app.favorite_tools(),
            vec![Tool::Rectangle, Tool::Cross, Tool::TrendLine]
        );

        // Move Rectangle from index 0 to index 1.
        d.app.move_favorite_tool(0, 1);
        assert_eq!(
            d.app.favorite_tools(),
            vec![Tool::Cross, Tool::Rectangle, Tool::TrendLine]
        );
    }

    #[test]
    fn test_persistence_after_reload() {
        let dir = std::env::temp_dir().join(format!("opentern-fav-persist-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = Store::open(&dir).unwrap();
        let mut app = App::with_context(
            &egui::Context::default(),
            crate::shell::Arrangement::default(),
            store,
            true,
        );

        // Customize favorites and toolbar.
        app.set_favorite_tools(&[Tool::Ray, Tool::Triangle, Tool::Brush]);
        app.layout.fav_toolbar_vertical = true;
        app.layout.fav_toolbar_pos = Some([123.0, 456.0]);
        app.layout.fav_toolbar_collapsed = true;
        app.save_fav_toolbar_settings();

        // Drop app and reopen from the same store.
        drop(app);
        let store2 = Store::open(&dir).unwrap();
        let app2 = App::with_context(
            &egui::Context::default(),
            crate::shell::Arrangement::default(),
            store2,
            true,
        );

        assert_eq!(
            app2.favorite_tools(),
            vec![Tool::Ray, Tool::Triangle, Tool::Brush],
            "favorites persisted"
        );
        assert!(app2.layout.fav_toolbar_vertical, "vertical orientation persisted");
        assert_eq!(app2.layout.fav_toolbar_pos, Some([123.0, 456.0]), "position persisted");
        assert!(app2.layout.fav_toolbar_collapsed, "collapsed state persisted");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_empty_favorites_state() {
        let dir = std::env::temp_dir().join(format!("opentern-fav-empty-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = Store::open(&dir).unwrap();
        let mut app = App::with_context(
            &egui::Context::default(),
            crate::shell::Arrangement::default(),
            store,
            true,
        );

        // Clear all favorites.
        app.set_favorite_tools(&[]);
        assert!(app.favorite_tools().is_empty());

        // Reopen from store: empty state must not re-seed defaults.
        drop(app);
        let store2 = Store::open(&dir).unwrap();
        let app2 = App::with_context(
            &egui::Context::default(),
            crate::shell::Arrangement::default(),
            store2,
            true,
        );
        assert!(
            app2.favorite_tools().is_empty(),
            "empty favorites state persists without re-seeding"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_tool_activation_and_escape() {
        let mut d = Driver::new("fav-activation");
        d.app.pick_tool(Tool::TrendLine);
        assert_eq!(d.app.layout.tool, Tool::TrendLine);

        // Picking the same tool cancels to Cross.
        d.app.pick_tool(Tool::Cross);
        assert_eq!(d.app.layout.tool, Tool::Cross);

        // Pick rectangle then press Escape.
        d.app.pick_tool(Tool::Rectangle);
        assert_eq!(d.app.layout.tool, Tool::Rectangle);
        d.key(Key::Escape, Modifiers::NONE);
        assert_eq!(d.app.layout.tool, Tool::Cross, "Escape cancels tool to cross");
    }

    #[test]
    fn test_driver_favorite_toolbar_rendering_and_layouts() {
        let mut d = Driver::new("fav-render");
        assert!(d.app.layout.fav_toolbar);

        // Render frame with single chart.
        d.frame(vec![]);
        assert!(d.app.chart_ui.main_rect.is_positive());

        // Switch to 2-chart grid.
        d.app.set_grid(Grid::TwoCols);
        d.frame(vec![]);
        assert!(d.app.chart_ui.main_rect.is_positive());

        // Switch to 4-chart grid.
        d.app.set_grid(Grid::Four);
        d.frame(vec![]);
        assert!(d.app.chart_ui.main_rect.is_positive());

        // Toggle vertical toolbar.
        d.app.layout.fav_toolbar_vertical = true;
        d.frame(vec![]);

        // Toggle collapsed toolbar.
        d.app.layout.fav_toolbar_collapsed = true;
        d.frame(vec![]);

        // Close toolbar.
        d.app.layout.fav_toolbar = false;
        d.frame(vec![]);
        assert!(!d.app.layout.fav_toolbar);
    }
}

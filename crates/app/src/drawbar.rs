//! The floating toolbar over the chart for the selected drawing: colours,
//! width, line style, text, lock, default style and delete. It looks like
//! the favourite drawings toolbar (`favbar.rs`) and keeps out of its way.

use crate::app::App;
use crate::chart::draw::Tool;
use crate::dialogs::color::color_button_with_tip;
use crate::i18n::{tr, trf};
use crate::icons::ph;
use crate::motion;
use crate::theme;
use crate::widgets as w;
use egui::{Align2, CornerRadius, CursorIcon, Id, Margin, Pos2, Rect, Sense, Stroke, pos2, vec2};
use opentern_series::study::LineStyle;

impl App {
    pub(crate) fn drawing_bar(&mut self, ctx: &egui::Context) {
        if self.modal.is_some() {
            return;
        }

        // Multi-drawing selection toolbar.
        if self.chart_ui.draw.multi_selected.len() > 1 {
            let t = self.t();
            let anchor = self.chart_ui.main_rect.center_top() + vec2(0.0, 44.0);
            let count = self.chart_ui.draw.multi_selected.len();
            let mut delete = false;
            let mut toggle_lock = false;
            let mut toggle_hide = false;
            egui::Area::new(egui::Id::new("multi-drawing-bar"))
                .order(egui::Order::Foreground)
                .pivot(egui::Align2::CENTER_TOP)
                .fixed_pos(anchor)
                .show(ctx, |ui| {
                    egui::Frame::new()
                        .fill(t.elevated)
                        .stroke(Stroke::new(1.0, t.border))
                        .corner_radius(CornerRadius::same(10))
                        .inner_margin(Margin::symmetric(8, 4))
                        .shadow(egui::Shadow {
                            offset: [0, 6],
                            blur: 18,
                            spread: 0,
                            color: t.shadow(),
                        })
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(
                                    egui::RichText::new(trf!("{} drawings selected", count))
                                        .font(theme::medium(12.5))
                                        .color(t.text),
                                );
                                ui.separator();
                                if crate::widgets::eased(ui, egui::Button::new(ph::LOCK_SIMPLE))
                                    .on_hover_text(tr("Lock / unlock"))
                                    .clicked()
                                {
                                    toggle_lock = true;
                                }
                                if crate::widgets::eased(ui, egui::Button::new(ph::EYE_SLASH))
                                    .on_hover_text(tr("Hide / show"))
                                    .clicked()
                                {
                                    toggle_hide = true;
                                }
                                if crate::widgets::eased(ui, egui::Button::new(ph::TRASH))
                                    .on_hover_text(tr("Remove (Delete)"))
                                    .clicked()
                                {
                                    delete = true;
                                }
                            });
                        });
                });
            if delete {
                self.delete_selected_drawing();
                return;
            }
            if toggle_lock {
                let ids = self.chart_ui.draw.multi_selected.clone();
                let all_locked = self
                    .chart
                    .drawings
                    .iter()
                    .filter(|d| ids.contains(&d.id))
                    .all(|d| d.locked);
                for d in self.chart.drawings.iter_mut().filter(|d| ids.contains(&d.id)) {
                    d.locked = !all_locked;
                }
                self.chart.drawings_changed += 1;
            }
            if toggle_hide {
                let ids = self.chart_ui.draw.multi_selected.clone();
                let all_hidden = self
                    .chart
                    .drawings
                    .iter()
                    .filter(|d| ids.contains(&d.id))
                    .all(|d| d.hidden);
                for d in self.chart.drawings.iter_mut().filter(|d| ids.contains(&d.id)) {
                    d.hidden = !all_hidden;
                }
                self.chart.drawings_changed += 1;
            }
            return;
        }

        let Some(id) = self.chart_ui.draw.selected else { return };
        let Some(d) = self.chart.drawings.iter().find(|d| d.id == id).cloned() else {
            self.chart_ui.draw.selected = None;
            return;
        };
        if !self.charts_on_screen(ctx) {
            return;
        }
        let t = self.t();
        let default_anchor = self.drawing_bar_anchor(ctx);
        let mut bar_pos = self.chart_ui.drawbar_pos.unwrap_or(default_anchor);
        let min_x = self.chart_ui.main_rect.left() + 8.0;
        let max_x = (self.chart_ui.main_rect.right() - 260.0).max(min_x);
        let min_y = self.chart_ui.main_rect.top() + 8.0;
        let max_y = (self.chart_ui.main_rect.bottom() - 50.0).max(min_y);
        bar_pos.x = bar_pos.x.clamp(min_x, max_x);
        bar_pos.y = bar_pos.y.clamp(min_y, max_y);
        let mut edit = d.clone();
        let mut delete = false;
        let mut save_default = false;
        let mut open_settings = false;
        let mut save_template_dialog = false;
        let mut pos_changed = false;

        egui::Area::new(egui::Id::new("drawing-bar"))
            .order(egui::Order::Foreground)
            .pivot(egui::Align2::CENTER_TOP)
            .fixed_pos(bar_pos)
            .show(ctx, |ui| {
                // The floating bars' surface (`favbar.rs`): the popups'
                // fill, edge and shadow, the same height.
                egui::Frame::new()
                    .fill(t.popup())
                    .stroke(Stroke::new(1.0, t.edge()))
                    .corner_radius(CornerRadius::same(12))
                    .inner_margin(Margin::symmetric(6, 5))
                    .shadow(ui.visuals().popup_shadow)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 4.0;
                            ui.set_min_height(BTN);
                            // Movable drag handle.
                            let (grip_r, grip_resp) = ui.allocate_exact_size(vec2(10.0, 24.0), Sense::drag());
                            let grip_hover = motion::hover(&grip_resp);
                            ui.painter().text(
                                grip_r.center(),
                                Align2::CENTER_CENTER,
                                ph::DOTS_SIX_VERTICAL,
                                theme::regular(14.0),
                                motion::lerp_color(t.text3, t.text, grip_hover),
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
                                bar_pos += delta;
                                pos_changed = true;
                            }

                            // Templates menu button (Image #34).
                            let tpl_resp =
                                crate::widgets::tool_button(ui, &t, ph::SQUARES_FOUR, tr("Templates"), false, 26.0);
                            let tpl_kind = format!("drawing:{}", d.tool.id());
                            let tpl_list = self.store.templates(&tpl_kind).to_vec();
                            egui::Popup::menu(&tpl_resp).show(|ui| {
                                ui.set_width(220.0);
                                ui.spacing_mut().item_spacing.y = 1.0;
                                if crate::widgets::menu_item(
                                    ui,
                                    &t,
                                    Some(ph::FLOPPY_DISK),
                                    tr("Save Drawing Template As..."),
                                    "",
                                    false,
                                )
                                .clicked()
                                {
                                    save_template_dialog = true;
                                    ui.close();
                                }
                                if crate::widgets::menu_item(
                                    ui,
                                    &t,
                                    Some(ph::ARROWS_COUNTER_CLOCKWISE),
                                    tr("Apply Default Drawing Template"),
                                    "",
                                    false,
                                )
                                .clicked()
                                {
                                    edit.style = crate::chart::draw::DrawStyle::for_tool(d.tool);
                                    ui.close();
                                }
                                if !tpl_list.is_empty() {
                                    crate::widgets::menu_rule(ui, &t);
                                    for tpl in &tpl_list {
                                        let (r, resp) =
                                            ui.allocate_exact_size(vec2(ui.available_width(), 28.0), Sense::click());
                                        let hover = motion::hover(&resp);
                                        if hover > 0.0 {
                                            ui.painter().rect_filled(
                                                r,
                                                CornerRadius::same(6),
                                                motion::with_alpha(t.hover_wash(), hover),
                                            );
                                        }
                                        ui.painter().text(
                                            pos2(r.left() + 10.0, r.center().y),
                                            Align2::LEFT_CENTER,
                                            &tpl.name,
                                            theme::regular(13.0),
                                            t.text,
                                        );
                                        let del_r = Rect::from_center_size(
                                            pos2(r.right() - 14.0, r.center().y),
                                            vec2(16.0, 16.0),
                                        );
                                        ui.painter().text(
                                            del_r.center(),
                                            Align2::CENTER_CENTER,
                                            ph::TRASH,
                                            theme::regular(12.0),
                                            t.text3,
                                        );
                                        if ui
                                            .interact(del_r, Id::new(("del-tpl", &tpl.name)), Sense::click())
                                            .clicked()
                                        {
                                            let _ = self.store.delete_template(&tpl_kind, &tpl.name);
                                        } else if resp.clicked() {
                                            if let Ok(st) =
                                                serde_json::from_str::<crate::chart::draw::DrawStyle>(&tpl.json)
                                            {
                                                edit.style = st;
                                            }
                                            ui.close();
                                        }
                                    }
                                }
                            });

                            ui.label(
                                egui::RichText::new(tr(d.tool.name()))
                                    .font(theme::medium(12.5))
                                    .color(t.text2),
                            );
                            w::vdivider(ui, &t);
                            if !matches!(d.tool, Tool::LongPosition | Tool::ShortPosition) {
                                color_button_with_tip(ui, &t, &mut edit.style.color, tr("Line color"));
                            }
                            let fills = matches!(
                                d.tool,
                                Tool::Rectangle
                                    | Tool::Ellipse
                                    | Tool::Triangle
                                    | Tool::ParallelChannel
                                    | Tool::PriceRange
                                    | Tool::DateRange
                                    | Tool::DatePriceRange
                            );
                            if fills {
                                color_button_with_tip(ui, &t, &mut edit.style.fill, tr("Background color"));
                            }
                            if matches!(d.tool, Tool::LongPosition | Tool::ShortPosition) {
                                let params = edit
                                    .style
                                    .position_params
                                    .get_or_insert_with(crate::chart::draw::PositionParams::default);
                                let mut text_c = params.text_color.unwrap_or(opentern_series::Rgba::hex(0xFFFFFF));
                                if color_button_with_tip(ui, &t, &mut text_c, tr("Text color")) {
                                    params.text_color = Some(text_c);
                                }
                                let mut tc = params.target_color.unwrap_or(opentern_series::Rgba::hex(0x089981));
                                if color_button_with_tip(ui, &t, &mut tc, tr("Target color")) {
                                    params.target_color = Some(tc);
                                }
                                let mut sc = params.stop_color.unwrap_or(opentern_series::Rgba::hex(0xF23645));
                                if color_button_with_tip(ui, &t, &mut sc, tr("Stop color")) {
                                    params.stop_color = Some(sc);
                                }
                            }
                            crate::widgets::combo("draw-width")
                                .width(50.0)
                                .selected_text(trf!("{}px", edit.style.width))
                                .show_ui(ui, |ui| {
                                    for w in 1..=4u8 {
                                        crate::widgets::option_value(ui, &mut edit.style.width, w, trf!("{w}px"));
                                    }
                                });
                            let name = |s: LineStyle| match s {
                                LineStyle::Solid => tr("Solid"),
                                LineStyle::Dashed => tr("Dashed"),
                                LineStyle::Dotted => tr("Dotted"),
                            };
                            crate::widgets::combo("draw-line")
                                .width(70.0)
                                .selected_text(name(edit.style.line))
                                .show_ui(ui, |ui| {
                                    for s in [LineStyle::Solid, LineStyle::Dashed, LineStyle::Dotted] {
                                        crate::widgets::option_value(ui, &mut edit.style.line, s, name(s));
                                    }
                                });
                            if matches!(
                                d.tool,
                                Tool::Text
                                    | Tool::ArrowUp
                                    | Tool::ArrowDown
                                    | Tool::TrendLine
                                    | Tool::Ray
                                    | Tool::ExtendedLine
                                    | Tool::Arrow
                                    | Tool::HorizontalLine
                                    | Tool::HorizontalRay
                                    | Tool::Rectangle
                            ) {
                                ui.add(
                                    crate::widgets::field(&mut edit.text)
                                        .hint_text(tr("Text"))
                                        .desired_width(120.0),
                                );
                            }
                            // Toggles and buttons as in the drawing toolbar.
                            let toggle = |ui: &mut egui::Ui, on: &mut bool, glyph: &str, tip: &str| {
                                if w::tool_button(ui, &t, glyph, tip, *on, BTN).clicked() {
                                    *on = !*on;
                                }
                            };
                            if matches!(d.tool, Tool::TrendLine | Tool::Arrow) {
                                toggle(ui, &mut edit.style.extend_left, ph::ARROW_LINE_LEFT, tr("Extend left"));
                                toggle(
                                    ui,
                                    &mut edit.style.extend_right,
                                    ph::ARROW_LINE_RIGHT,
                                    tr("Extend right"),
                                );
                            }
                            if d.tool == Tool::Rectangle {
                                ui.toggle_value(&mut edit.style.middle_line, ph::EQUALS)
                                    .on_hover_text(tr("Middle line"));
                            }
                            if matches!(d.tool, Tool::FibRetracement | Tool::FibExtension) {
                                toggle(
                                    ui,
                                    &mut edit.style.extend_right,
                                    ph::ARROW_LINE_RIGHT,
                                    tr("Extend lines right"),
                                );
                            }
                            if matches!(
                                d.tool,
                                Tool::HorizontalLine | Tool::HorizontalRay | Tool::FibRetracement | Tool::FibExtension
                            ) {
                                toggle(ui, &mut edit.style.labels, ph::TAG, tr("Price labels"));
                            }
                            w::vdivider(ui, &t);
                            if w::tool_button(ui, &t, ph::GEAR, tr("Settings"), false, BTN).clicked() {
                                open_settings = true;
                            }
                            let lock = if edit.locked {
                                ph::LOCK_SIMPLE
                            } else {
                                ph::LOCK_SIMPLE_OPEN
                            };
                            toggle(ui, &mut edit.locked, lock, "");
                            if w::tool_button(
                                ui,
                                &t,
                                ph::STAR,
                                tr("Use this style for new drawings of this kind"),
                                false,
                                BTN,
                            )
                            .clicked()
                            {
                                save_default = true;
                            }
                            if w::tool_button(ui, &t, ph::TRASH, tr("Remove (Delete)"), false, BTN).clicked() {
                                delete = true;
                            }
                        });
                    });
            });
        if pos_changed {
            self.chart_ui.drawbar_pos = Some(bar_pos);
        }
        if delete {
            self.delete_selected_drawing();
            return;
        }
        if save_template_dialog {
            self.modal = Some(crate::dialogs::Modal::SaveTemplate {
                kind: crate::dialogs::TemplateKind::Drawing(d.tool.id().to_string()),
                name: String::new(),
            });
            return;
        }
        if open_settings {
            self.modal =
                crate::dialogs::DrawingSettings::new(self, id).map(|s| crate::dialogs::Modal::Drawing(Box::new(s)));
        }
        if save_default {
            self.save_drawing_default(d.tool, &edit.style);
        }
        if edit != d {
            self.edit_drawing(id, |x| *x = edit);
        }
    }
}

/// Its buttons' side, as on the favourite drawings toolbar.
const BTN: f32 = 32.0;

impl App {
    /// Where the bar goes (its top centre): over the top of the chart,
    /// unless the favourite drawings toolbar is there; then below, above
    /// or beside that, whichever fits first.
    fn drawing_bar_anchor(&self, ctx: &egui::Context) -> Pos2 {
        let usual = self.chart_ui.main_rect.center_top() + vec2(0.0, 44.0);
        let fav = self.favbar.at.bar;
        if !fav.is_positive() {
            return usual;
        }
        // Its size as last drawn (a guess before then).
        let size = ctx
            .memory(|m| m.area_rect(egui::Id::new("drawing-bar")))
            .map_or(vec2(420.0, 42.0), |r| r.size());
        let rect = |top_centre: Pos2| Rect::from_min_size(top_centre - vec2(size.x / 2.0, 0.0), size);
        const GAP: f32 = 8.0;
        let clear = fav.expand(GAP / 2.0);
        let room = self.grid_rect;
        let x = usual.x;
        [
            usual,
            Pos2::new(x, fav.bottom() + GAP),
            Pos2::new(x, fav.top() - GAP - size.y),
            Pos2::new(fav.right() + GAP + size.x / 2.0, usual.y),
            Pos2::new(fav.left() - GAP - size.x / 2.0, usual.y),
        ]
        .into_iter()
        .find(|&p| {
            let r = rect(p);
            room.contains_rect(r) && !r.intersects(clear)
        })
        .unwrap_or(usual)
    }
}

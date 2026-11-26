//! The floating toolbar over the chart for the selected drawing: colours,
//! width, line style, text, lock, default style and delete.

use crate::app::App;
use crate::chart::draw::Tool;
use crate::dialogs::color::color_button_with_tip;
use crate::i18n::{tr, trf};
use crate::icons::ph;
use crate::theme;
use egui::{CornerRadius, Margin, Stroke, vec2};
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
        let t = self.t();
        let anchor = self.chart_ui.main_rect.center_top() + vec2(0.0, 44.0);
        let mut edit = d.clone();
        let mut delete = false;
        let mut save_default = false;
        let mut open_settings = false;
        egui::Area::new(egui::Id::new("drawing-bar"))
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
                                egui::RichText::new(tr(d.tool.name()))
                                    .font(theme::medium(12.5))
                                    .color(t.text2),
                            );
                            ui.separator();
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
                            if matches!(d.tool, Tool::TrendLine | Tool::Arrow) {
                                ui.toggle_value(&mut edit.style.extend_left, ph::ARROW_LINE_LEFT)
                                    .on_hover_text(tr("Extend left"));
                                ui.toggle_value(&mut edit.style.extend_right, ph::ARROW_LINE_RIGHT)
                                    .on_hover_text(tr("Extend right"));
                            }
                            if d.tool == Tool::Rectangle {
                                ui.toggle_value(&mut edit.style.middle_line, ph::EQUALS)
                                    .on_hover_text(tr("Middle line"));
                            }
                            if matches!(d.tool, Tool::FibRetracement | Tool::FibExtension) {
                                ui.toggle_value(&mut edit.style.extend_right, ph::ARROW_LINE_RIGHT)
                                    .on_hover_text(tr("Extend lines right"));
                            }
                            if matches!(
                                d.tool,
                                Tool::HorizontalLine | Tool::HorizontalRay | Tool::FibRetracement | Tool::FibExtension
                            ) {
                                ui.toggle_value(&mut edit.style.labels, ph::TAG)
                                    .on_hover_text(tr("Price labels"));
                            }
                            ui.separator();
                            if crate::widgets::eased(ui, egui::Button::new(ph::GEAR))
                                .on_hover_text(tr("Settings"))
                                .clicked()
                            {
                                open_settings = true;
                            }
                            let lock = if edit.locked {
                                ph::LOCK_SIMPLE
                            } else {
                                ph::LOCK_SIMPLE_OPEN
                            };
                            ui.toggle_value(&mut edit.locked, lock);
                            if crate::widgets::eased(ui, egui::Button::new(ph::STAR))
                                .on_hover_text(tr("Use this style for new drawings of this kind"))
                                .clicked()
                            {
                                save_default = true;
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

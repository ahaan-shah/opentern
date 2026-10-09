//! The floating toolbar over the chart for the selected drawing: colours,
//! width, line style, text, lock, default style and delete.

use crate::app::App;
use crate::chart::draw::Tool;
use crate::dialogs::color::color_button;
use crate::icons::ph;
use crate::theme;
use egui::{CornerRadius, Margin, Stroke, vec2};
use opentern_series::study::LineStyle;

impl App {
    pub(crate) fn drawing_bar(&mut self, ctx: &egui::Context) {
        let Some(id) = self.chart_ui.draw.selected else { return };
        let Some(d) = self.chart.drawings.iter().find(|d| d.id == id).cloned() else {
            self.chart_ui.draw.selected = None;
            return;
        };
        if self.modal.is_some() {
            return;
        }
        let t = self.t();
        let anchor = self.chart_ui.main_rect.center_top() + vec2(0.0, 44.0);
        let mut edit = d.clone();
        let mut delete = false;
        let mut save_default = false;
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
                                egui::RichText::new(d.tool.name())
                                    .font(theme::medium(12.5))
                                    .color(t.text2),
                            );
                            ui.separator();
                            color_button(ui, &t, &mut edit.style.color);
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
                                color_button(ui, &t, &mut edit.style.fill);
                            }
                            egui::ComboBox::from_id_salt("draw-width")
                                .width(50.0)
                                .selected_text(format!("{}px", edit.style.width))
                                .show_ui(ui, |ui| {
                                    for w in 1..=4u8 {
                                        ui.selectable_value(&mut edit.style.width, w, format!("{w}px"));
                                    }
                                });
                            let name = |s: LineStyle| match s {
                                LineStyle::Solid => "Solid",
                                LineStyle::Dashed => "Dashed",
                                LineStyle::Dotted => "Dotted",
                            };
                            egui::ComboBox::from_id_salt("draw-line")
                                .width(70.0)
                                .selected_text(name(edit.style.line))
                                .show_ui(ui, |ui| {
                                    for s in [LineStyle::Solid, LineStyle::Dashed, LineStyle::Dotted] {
                                        ui.selectable_value(&mut edit.style.line, s, name(s));
                                    }
                                });
                            if matches!(d.tool, Tool::Text | Tool::ArrowUp | Tool::ArrowDown) {
                                ui.add(
                                    egui::TextEdit::singleline(&mut edit.text)
                                        .hint_text("Text")
                                        .desired_width(140.0),
                                );
                            }
                            if matches!(d.tool, Tool::TrendLine | Tool::Arrow) {
                                ui.toggle_value(&mut edit.style.extend_left, ph::ARROW_LINE_LEFT)
                                    .on_hover_text("Extend left");
                                ui.toggle_value(&mut edit.style.extend_right, ph::ARROW_LINE_RIGHT)
                                    .on_hover_text("Extend right");
                            }
                            if matches!(d.tool, Tool::FibRetracement | Tool::FibExtension) {
                                ui.toggle_value(&mut edit.style.extend_right, ph::ARROW_LINE_RIGHT)
                                    .on_hover_text("Extend lines right");
                            }
                            if matches!(
                                d.tool,
                                Tool::HorizontalLine | Tool::HorizontalRay | Tool::FibRetracement | Tool::FibExtension
                            ) {
                                ui.toggle_value(&mut edit.style.labels, ph::TAG)
                                    .on_hover_text("Price labels");
                            }
                            ui.separator();
                            let lock = if edit.locked {
                                ph::LOCK_SIMPLE
                            } else {
                                ph::LOCK_SIMPLE_OPEN
                            };
                            ui.toggle_value(&mut edit.locked, lock).on_hover_text("Lock");
                            if ui
                                .button(ph::STAR)
                                .on_hover_text("Use this style for new drawings of this kind")
                                .clicked()
                            {
                                save_default = true;
                            }
                            if ui.button(ph::TRASH).on_hover_text("Remove (Delete)").clicked() {
                                delete = true;
                            }
                        });
                    });
            });
        if delete {
            self.delete_selected_drawing();
            return;
        }
        if save_default {
            self.save_drawing_default(d.tool, &edit.style);
        }
        if edit != d {
            self.edit_drawing(id, |x| *x = edit);
        }
    }
}

//! The floating toolbar over the chart for the selected drawing: colours,
//! width, line style, text, lock, default style and delete. It looks like
//! the favourite drawings toolbar (`favbar.rs`) and keeps out of its way.

use crate::app::App;
use crate::chart::draw::Tool;
use crate::dialogs::color::color_button;
use crate::i18n::{tr, trf};
use crate::icons::ph;
use crate::theme;
use crate::widgets as w;
use egui::{CornerRadius, Margin, Pos2, Rect, Stroke, vec2};
use opentern_series::study::LineStyle;

impl App {
    pub(crate) fn drawing_bar(&mut self, ctx: &egui::Context) {
        let Some(id) = self.chart_ui.draw.selected else { return };
        let Some(d) = self.chart.drawings.iter().find(|d| d.id == id).cloned() else {
            self.chart_ui.draw.selected = None;
            return;
        };
        if !self.charts_on_screen(ctx) {
            return;
        }
        let t = self.t();
        let anchor = self.drawing_bar_anchor(ctx);
        let mut edit = d.clone();
        let mut delete = false;
        let mut save_default = false;
        egui::Area::new(egui::Id::new("drawing-bar"))
            .order(egui::Order::Foreground)
            .pivot(egui::Align2::CENTER_TOP)
            .fixed_pos(anchor)
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
                            ui.add_space(6.0);
                            ui.label(
                                egui::RichText::new(tr(d.tool.name()))
                                    .font(theme::medium(12.5))
                                    .color(t.text2),
                            );
                            w::vdivider(ui, &t);
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
                            if matches!(d.tool, Tool::Text | Tool::ArrowUp | Tool::ArrowDown) {
                                ui.add(
                                    crate::widgets::field(&mut edit.text)
                                        .hint_text(tr("Text"))
                                        .desired_width(140.0),
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

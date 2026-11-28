//! The floating toolbar over the chart for the selected drawing: colours,
//! width, line style, text, lock, default style and delete. It looks like
//! the favourite drawings toolbar (`favbar.rs`) and keeps out of its way.

use crate::app::App;
use crate::chart::draw::{Along, DrawStyle, FIB_LEVELS, FibLevel, Side, TextAt, Tool};
use crate::dialogs::color::color_button;
use crate::dialogs::{Modal, TemplateKind};
use crate::i18n::{tr, trc, trf};
use crate::icons::ph;
use crate::theme;
use crate::widgets as w;
use egui::{CornerRadius, Margin, Pos2, Rect, Stroke, vec2};
use opentern_series::Rgba;
use opentern_series::study::LineStyle;

impl App {
    pub(crate) fn drawing_bar(&mut self, ctx: &egui::Context) {
        let Some(id) = self.chart_ui.draw.selected else { return };
        let Some(d) = self.chart.drawings.iter().find(|d| d.id == id).cloned() else {
            self.chart_ui.draw.select(None);
            return;
        };
        if !self.charts_on_screen(ctx) {
            return;
        }
        if !self.chart_ui.draw.also.is_empty() {
            self.several_bar(ctx);
            return;
        }
        let t = self.t();
        let anchor = self.drawing_bar_anchor(ctx);
        let mut edit = d.clone();
        let mut delete = false;
        let mut save_default = false;
        // The Fib levels panel, open for this drawing only. A panel, not a
        // popup: egui keeps one popup open, and each level's colour is one.
        let fib = matches!(d.tool, Tool::FibRetracement | Tool::FibExtension);
        let levels_key = egui::Id::new("fib-levels-open");
        let mut levels_open = fib && ctx.data(|m| m.get_temp::<u64>(levels_key)) == Some(id);
        let mut levels_x = anchor.x;
        // The Text panel (lines and boxes), the same way.
        let text_key = egui::Id::new("drawing-text-open");
        let carries = d.tool.carries_text();
        let mut text_open = carries && ctx.data(|m| m.get_temp::<u64>(text_key)) == Some(id);
        let mut text_x = anchor.x;
        let templates: Vec<String> = self
            .store
            .templates(&TemplateKind::Drawing(d.tool).key())
            .iter()
            .map(|t| t.name.clone())
            .collect();
        let mut template = None;
        let bar = egui::Area::new(egui::Id::new("drawing-bar"))
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
                            // Toggles and buttons as in the drawing toolbar.
                            let toggle = |ui: &mut egui::Ui, on: &mut bool, glyph: &str, tip: &str| {
                                if w::tool_button(ui, &t, glyph, tip, *on, BTN).clicked() {
                                    *on = !*on;
                                }
                            };
                            if d.tool == Tool::Text {
                                // Text has no line: its size and weight instead.
                                text_size(ui, &mut edit.style.font);
                                toggle(ui, &mut edit.style.bold, ph::TEXT_B, tr("Bold"));
                            } else {
                                width_and_line(ui, &mut edit.style);
                            }
                            if matches!(d.tool, Tool::Text | Tool::ArrowUp | Tool::ArrowDown) {
                                ui.add(
                                    crate::widgets::field(&mut edit.text)
                                        .hint_text(tr("Text"))
                                        .desired_width(140.0),
                                );
                            }
                            if carries {
                                let r = w::tool_button(
                                    ui,
                                    &t,
                                    ph::TEXT_T,
                                    tr("Text"),
                                    text_open || !edit.text.is_empty(),
                                    BTN,
                                );
                                if r.clicked() {
                                    text_open = !text_open;
                                    levels_open = false;
                                }
                                text_x = r.rect.left();
                            }
                            if matches!(d.tool, Tool::TrendLine | Tool::Arrow) {
                                toggle(ui, &mut edit.style.extend_left, ph::ARROW_LINE_LEFT, tr("Extend left"));
                                toggle(
                                    ui,
                                    &mut edit.style.extend_right,
                                    ph::ARROW_LINE_RIGHT,
                                    tr("Extend right"),
                                );
                            }
                            if fib {
                                toggle(
                                    ui,
                                    &mut edit.style.extend_right,
                                    ph::ARROW_LINE_RIGHT,
                                    tr("Extend lines right"),
                                );
                                let r = w::tool_button(ui, &t, ph::LIST_NUMBERS, tr("Levels"), levels_open, BTN);
                                if r.clicked() {
                                    levels_open = !levels_open;
                                    text_open = false;
                                }
                                levels_x = r.rect.left();
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
                            let r = w::tool_button(ui, &t, ph::PALETTE, tr("Template"), false, BTN);
                            self.still_open(ui, &r, "drawing-template");
                            if r.clicked() {
                                // The menu would sit over the panels.
                                levels_open = false;
                                text_open = false;
                            }
                            egui::Popup::menu(&r)
                                .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                                .show(|ui| {
                                    templates_menu(ui, &t, &templates, &mut template);
                                    if template.is_some() {
                                        ui.close();
                                    }
                                });
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
        if levels_open {
            ctx.data_mut(|m| m.insert_temp(levels_key, id));
            let at = Pos2::new(levels_x, bar.response.rect.bottom() + 6.0);
            fib_levels_panel(ctx, &t, at, d.tool, &mut edit.style);
        } else {
            ctx.data_mut(|m| m.remove_temp::<u64>(levels_key));
        }
        if text_open {
            ctx.data_mut(|m| m.insert_temp(text_key, id));
            let at = Pos2::new(text_x, bar.response.rect.bottom() + 6.0);
            text_panel(ctx, &t, at, d.tool, &mut edit);
        } else {
            ctx.data_mut(|m| m.remove_temp::<u64>(text_key));
        }
        if delete {
            self.delete_selected_drawing();
            return;
        }
        if save_default {
            self.save_drawing_default(d.tool, &edit.style);
        }
        match template {
            Some(Pick::Save) => {
                // Saved from this drawing as it now stands.
                if edit != d {
                    self.edit_drawing(id, |x| *x = edit.clone());
                }
                self.modal = Some(Modal::SaveTemplate {
                    kind: TemplateKind::Drawing(d.tool),
                    name: String::new(),
                });
                return;
            }
            Some(Pick::Apply(name)) => {
                if let Some(style) = self.drawing_template(d.tool, &name) {
                    edit.style = style;
                }
            }
            Some(Pick::Delete(name)) => self.delete_template(TemplateKind::Drawing(d.tool), &name),
            Some(Pick::Reset) => {
                edit.style = DrawStyle::for_tool(d.tool);
                self.reset_drawing_default(d.tool);
            }
            None => {}
        }
        if edit != d {
            self.edit_drawing(id, |x| *x = edit);
        }
    }
}

impl App {
    /// The bar while several drawings are selected: how many, one colour
    /// for all, lock and delete.
    fn several_bar(&mut self, ctx: &egui::Context) {
        let t = self.t();
        let ids = self.chart_ui.draw.all_selected();
        let picked: Vec<_> = self.chart.drawings.iter().filter(|d| ids.contains(&d.id)).collect();
        let Some(first) = picked.first() else { return };
        let mut color = first.style.color;
        let mut locked = picked.iter().all(|d| d.locked);
        let n = picked.len();
        let (was_color, was_locked) = (color, locked);
        let mut delete = false;
        egui::Area::new(egui::Id::new("drawing-bar"))
            .order(egui::Order::Foreground)
            .pivot(egui::Align2::CENTER_TOP)
            .fixed_pos(self.drawing_bar_anchor(ctx))
            .show(ctx, |ui| {
                bar_frame(ui, &t, |ui| {
                    ui.label(
                        egui::RichText::new(trf!("{n} drawings"))
                            .font(theme::medium(12.5))
                            .color(t.text2),
                    );
                    w::vdivider(ui, &t);
                    color_button(ui, &t, &mut color);
                    w::vdivider(ui, &t);
                    let lock = if locked { ph::LOCK_SIMPLE } else { ph::LOCK_SIMPLE_OPEN };
                    if w::tool_button(ui, &t, lock, tr("Lock"), locked, BTN).clicked() {
                        locked = !locked;
                    }
                    if w::tool_button(ui, &t, ph::TRASH, tr("Remove (Delete)"), false, BTN).clicked() {
                        delete = true;
                    }
                });
            });
        if delete {
            self.delete_selected_drawing();
            return;
        }
        if color != was_color || locked != was_locked {
            self.chart.push_undo();
            for d in self.chart.drawings.iter_mut().filter(|d| ids.contains(&d.id)) {
                if color != was_color {
                    // The fill keeps its own see-through amount.
                    let alpha = d.style.fill.a() as u32;
                    d.style.color = color;
                    d.style.fill = Rgba((color.0 & 0xFFFF_FF00) | alpha);
                }
                d.locked = locked;
            }
            self.chart.drawings_changed += 1;
        }
    }
}

/// The floating bars' surface (`favbar.rs`): the popups' fill, edge and
/// shadow, the same height.
fn bar_frame(ui: &mut egui::Ui, t: &theme::Theme, add: impl FnOnce(&mut egui::Ui)) {
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
                add(ui);
            });
        });
}

/// A line's width and style.
pub(crate) fn width_and_line(ui: &mut egui::Ui, style: &mut DrawStyle) {
    w::combo("draw-width")
        .width(50.0)
        .selected_text(trf!("{}px", style.width))
        .show_ui(ui, |ui| {
            for x in 1..=4u8 {
                w::option_value(ui, &mut style.width, x, trf!("{x}px"));
            }
        });
    let name = |s: LineStyle| match s {
        LineStyle::Solid => tr("Solid"),
        LineStyle::Dashed => tr("Dashed"),
        LineStyle::Dotted => tr("Dotted"),
    };
    w::combo("draw-line")
        .width(70.0)
        .selected_text(name(style.line))
        .show_ui(ui, |ui| {
            for s in [LineStyle::Solid, LineStyle::Dashed, LineStyle::Dotted] {
                w::option_value(ui, &mut style.line, s, name(s));
            }
        });
}

/// The text's size, as TradingView offers them.
const TEXT_SIZES: [u8; 11] = [10, 11, 12, 14, 16, 20, 24, 28, 32, 40, 56];

pub(crate) fn text_size(ui: &mut egui::Ui, font: &mut u8) {
    w::combo("draw-text-size")
        .width(54.0)
        .selected_text(font.to_string())
        .show_ui(ui, |ui| {
            // Sized by the handle in between: the list still shows it.
            let mut sizes = TEXT_SIZES.to_vec();
            if !sizes.contains(font) {
                sizes.push(*font);
                sizes.sort();
            }
            for s in sizes {
                w::option_value(ui, font, s, s.to_string());
            }
        });
}

/// The Text panel under the bar for lines and boxes: the text, its size
/// and weight, and where it sits.
fn text_panel(ctx: &egui::Context, t: &theme::Theme, at: Pos2, tool: Tool, d: &mut crate::chart::draw::Drawing) {
    egui::Area::new(egui::Id::new("drawing-text"))
        .order(egui::Order::Foreground)
        .fixed_pos(at)
        .show(ctx, |ui| {
            egui::Frame::new()
                .fill(t.popup())
                .stroke(Stroke::new(1.0, t.edge()))
                .corner_radius(CornerRadius::same(12))
                .inner_margin(Margin::same(10))
                .shadow(ui.visuals().popup_shadow)
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing = vec2(4.0, 8.0);
                    let field = ui.add(w::field(&mut d.text).hint_text(tr("Text")).desired_width(236.0));
                    if ui.memory(|m| m.focused().is_none()) && d.text.is_empty() {
                        field.request_focus();
                    }
                    let style = &mut d.style;
                    let mut place = style.text_at(tool);
                    ui.horizontal(|ui| {
                        text_size(ui, &mut style.font);
                        if w::tool_button(ui, t, ph::TEXT_B, tr("Bold"), style.bold, 28.0).clicked() {
                            style.bold = !style.bold;
                        }
                        w::vdivider(ui, t);
                        for (along, glyph, tip) in [
                            (Along::Start, ph::TEXT_ALIGN_LEFT, tr("Left")),
                            (Along::Middle, ph::TEXT_ALIGN_CENTER, trc("text", "Middle")),
                            (Along::End, ph::TEXT_ALIGN_RIGHT, tr("Right")),
                        ] {
                            if w::tool_button(ui, t, glyph, tip, place.along == along, 28.0).clicked() {
                                place.along = along;
                            }
                        }
                        w::vdivider(ui, t);
                        let inside = if tool == Tool::Rectangle {
                            tr("Inside")
                        } else {
                            tr("On the line")
                        };
                        for (side, glyph, tip) in [
                            (Side::Above, ph::ALIGN_TOP, tr("Above")),
                            (Side::On, ph::ALIGN_CENTER_VERTICAL, inside),
                            (Side::Below, ph::ALIGN_BOTTOM, tr("Below")),
                        ] {
                            if w::tool_button(ui, t, glyph, tip, place.side == side, 28.0).clicked() {
                                place.side = side;
                            }
                        }
                    });
                    if place != style.text_at(tool) {
                        style.text_at = (place != TextAt::usual(tool)).then_some(place);
                    }
                });
        });
}

/// Its buttons' side, as on the favourite drawings toolbar.
const BTN: f32 = 32.0;

/// What was picked in the Template menu.
enum Pick {
    Save,
    Apply(String),
    Delete(String),
    Reset,
}

/// The drawing's Template menu: save its style under a name, apply or
/// delete a saved one, or go back to the standard style.
fn templates_menu(ui: &mut egui::Ui, t: &theme::Theme, names: &[String], pick: &mut Option<Pick>) {
    ui.set_width(240.0);
    ui.spacing_mut().item_spacing.y = 1.0;
    if w::menu_item(ui, t, Some(ph::FLOPPY_DISK), tr("Save template as"), "", false).clicked() {
        *pick = Some(Pick::Save);
    }
    if w::menu_item(
        ui,
        t,
        Some(ph::ARROW_COUNTER_CLOCKWISE),
        tr("Reset to default"),
        "",
        false,
    )
    .on_hover_text(tr("This drawing and new ones go back to the standard style"))
    .clicked()
    {
        *pick = Some(Pick::Reset);
    }
    if !names.is_empty() {
        w::menu_rule(ui, t);
    }
    for n in names {
        let resp = w::menu_item_trailing(ui, t, None, n, "", false, 28.0);
        let bin = Rect::from_center_size(resp.rect.right_center() - vec2(16.0, 0.0), vec2(24.0, 24.0));
        let b = ui.interact(bin, resp.id.with("del"), egui::Sense::click());
        let near = crate::motion::hover_if(&resp, resp.hovered() || b.hovered());
        if near > 0.0 {
            ui.painter().text(
                bin.center(),
                egui::Align2::CENTER_CENTER,
                ph::TRASH,
                theme::regular(14.0),
                crate::motion::with_alpha(
                    crate::motion::lerp_color(t.text3, t.neg, crate::motion::hover(&b)),
                    near,
                ),
            );
        }
        if b.on_hover_text(tr("Delete")).clicked() {
            *pick = Some(Pick::Delete(n.clone()));
        } else if resp.clicked() {
            *pick = Some(Pick::Apply(n.clone()));
        }
    }
}

/// The Fib levels under the bar: each one on or off, its value and colour,
/// remove it; add a level by its value; back to the standard ones.
fn fib_levels_panel(ctx: &egui::Context, t: &theme::Theme, at: Pos2, tool: Tool, style: &mut DrawStyle) {
    let mut levels = style.fib_levels(tool);
    let before = levels.clone();
    let new_key = egui::Id::new("fib-level-new");
    let mut typed: String = ctx.data(|m| m.get_temp(new_key)).unwrap_or_default();
    egui::Area::new(egui::Id::new("fib-levels"))
        .order(egui::Order::Foreground)
        .fixed_pos(at)
        .show(ctx, |ui| {
            egui::Frame::new()
                .fill(t.popup())
                .stroke(Stroke::new(1.0, t.edge()))
                .corner_radius(CornerRadius::same(12))
                .inner_margin(Margin::same(10))
                .shadow(ui.visuals().popup_shadow)
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing = vec2(6.0, 4.0);
                    let mut remove = None;
                    // As tall as the window allows below the bar, so the
                    // standard set shows whole on most screens.
                    let room = (ctx.content_rect().bottom() - at.y - 110.0).max(140.0);
                    egui::ScrollArea::vertical()
                        .max_height(room)
                        .auto_shrink([true, true])
                        .show(ui, |ui| {
                            for (i, l) in levels.iter_mut().enumerate() {
                                ui.push_id(i, |ui| {
                                    ui.horizontal(|ui| {
                                        w::checkbox(ui, &mut l.on, "");
                                        w::number(ui, ui.id().with("value"), &mut l.value, 0.01, -100.0, 100.0, 3);
                                        color_button(ui, t, &mut l.color);
                                        if w::tool_button(ui, t, ph::X, tr("Remove"), false, 26.0).clicked() {
                                            remove = Some(i);
                                        }
                                    });
                                });
                            }
                        });
                    if let Some(i) = remove {
                        levels.remove(i);
                    }
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        let field = ui.add(w::field(&mut typed).hint_text(tr("Add level")).desired_width(110.0));
                        let enter = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                        let add = w::tool_button(ui, t, ph::PLUS, tr("Add level"), false, 26.0).clicked();
                        if (enter || add)
                            && let Some(value) = parse_level(&typed)
                        {
                            let col = FIB_LEVELS[levels.len() % FIB_LEVELS.len()].1;
                            levels.push(FibLevel {
                                value,
                                color: Rgba::hex(col),
                                on: true,
                            });
                            levels.sort_by(|a, b| a.value.total_cmp(&b.value));
                            typed.clear();
                        }
                    });
                    ui.add_space(2.0);
                    if w::text_link(ui, t, tr("Reset levels")).clicked() {
                        levels = FibLevel::standard(tool);
                    }
                });
        });
    ctx.data_mut(|m| m.insert_temp(new_key, typed));
    if levels != before {
        // The standard set is stored as none, so it follows the defaults.
        style.levels = (levels != FibLevel::standard(tool)).then_some(levels);
    }
}

/// A typed level: "1.272", or "1,272" where a comma is the decimal mark.
fn parse_level(s: &str) -> Option<f64> {
    s.trim().replace(',', ".").parse::<f64>().ok().filter(|v| v.is_finite())
}

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

#[cfg(test)]
mod tests {
    use crate::chart::draw::{DrawStyle, Tool};
    use crate::drive::Driver;
    use crate::i18n::tr;
    use crate::icons::ph;
    use egui::{Event, Key, Modifiers};

    fn open_templates(d: &mut Driver) {
        let at = d
            .labelled_rects()
            .into_iter()
            .find(|(s, _, _)| s == ph::PALETTE)
            .map(|(_, r, _)| r.center())
            .expect("the Template button");
        d.click(at);
    }

    fn style(d: &Driver, id: u64) -> DrawStyle {
        d.app.chart.drawings.iter().find(|x| x.id == id).unwrap().style.clone()
    }

    /// Two styles saved for one Fib, applied back by name, kept across a
    /// restart; Reset to default brings back the standard style, also for
    /// new drawings.
    #[test]
    fn drawing_templates_save_apply_and_reset() {
        let mut d = Driver::new("drawing-templates");
        d.pick(Tool::FibRetracement);
        d.click(d.at(0.3, 0.3));
        d.click(d.at(0.6, 0.7));
        d.key(Key::Escape, Modifiers::NONE);
        let id = d.app.chart.drawings.last().expect("drawn").id;
        d.app.chart_ui.draw.selected = Some(id);
        let mut swing = style(&d, id);
        swing.width = 3;
        swing.levels = Some(
            crate::chart::draw::FibLevel::standard(Tool::FibRetracement)
                .into_iter()
                .filter(|l| [0.382, 0.5, 0.618].contains(&l.value))
                .collect(),
        );
        let s = swing.clone();
        d.app.edit_drawing(id, |x| x.style = s);
        d.frame(vec![]);

        open_templates(&mut d);
        let save = d.text_rect(tr("Save template as")).expect("Save template as").center();
        d.click(save);
        assert!(d.app.modal.is_some(), "asks for a name");
        d.frame(vec![]);
        d.frame(vec![Event::Text("Swing".into())]);
        d.key(Key::Enter, Modifiers::NONE);
        assert!(d.app.modal.is_none(), "saved");
        assert_eq!(
            d.app.drawing_template(Tool::FibRetracement, "Swing"),
            Some(swing.clone())
        );
        // Not offered to the extension.
        assert!(d.app.drawing_template(Tool::FibExtension, "Swing").is_none());

        // Something else, then the template by name.
        d.app.edit_drawing(id, |x| x.style.width = 1);
        d.app.chart_ui.draw.selected = Some(id);
        d.frame(vec![]);
        open_templates(&mut d);
        let item = d.text_rect("Swing").expect("listed").center();
        d.click(item);
        assert_eq!(style(&d, id), swing);

        // The star, then Reset to default: this one and new ones are standard.
        d.app.save_drawing_default(Tool::FibRetracement, &swing);
        open_templates(&mut d);
        let reset = d.text_rect(tr("Reset to default")).expect("Reset to default").center();
        d.click(reset);
        assert_eq!(style(&d, id), DrawStyle::for_tool(Tool::FibRetracement));
        assert!(!d.app.draw_styles.contains_key(&Tool::FibRetracement));
        assert!(d.app.store.templates("drawing").is_empty());

        // Named templates outlive a restart.
        let dir = d.dir.clone();
        d.app.on_exit_for_tests();
        std::mem::forget(d);
        let d = Driver::open(dir);
        assert_eq!(d.app.drawing_template(Tool::FibRetracement, "Swing"), Some(swing));
        assert!(!d.app.draw_styles.contains_key(&Tool::FibRetracement));
    }
}

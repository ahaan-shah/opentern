//! The floating toolbar over the chart for the selected drawing: colours,
//! width, line style, text, lock, default style and delete. It looks like
//! the favourite drawings toolbar (`favbar.rs`) and keeps out of its way.

use crate::app::App;
use crate::chart::draw::{DrawStyle, FIB_LEVELS, FibLevel, Tool};
use crate::dialogs::color::color_button;
use crate::dialogs::{Modal, TemplateKind};
use crate::i18n::{tr, trf};
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
        // The Fib levels panel, open for this drawing only. A panel, not a
        // popup: egui keeps one popup open, and each level's colour is one.
        let fib = matches!(d.tool, Tool::FibRetracement | Tool::FibExtension);
        let levels_key = egui::Id::new("fib-levels-open");
        let mut levels_open = fib && ctx.data(|m| m.get_temp::<u64>(levels_key)) == Some(id);
        let mut levels_x = anchor.x;
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
                            if r.clicked() {
                                // The menu would sit over the levels panel.
                                levels_open = false;
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
                    egui::ScrollArea::vertical()
                        .max_height(340.0)
                        .auto_shrink([true, true])
                        .show(ui, |ui| {
                            for (i, l) in levels.iter_mut().enumerate() {
                                ui.push_id(i, |ui| {
                                    ui.horizontal(|ui| {
                                        w::checkbox(ui, &mut l.on, "");
                                        ui.spacing_mut().interact_size.x = 64.0;
                                        ui.add(egui::DragValue::new(&mut l.value).speed(0.001).max_decimals(4));
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
        style.levels = if levels == FibLevel::standard(tool) {
            Vec::new()
        } else {
            levels
        };
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
        swing.levels = crate::chart::draw::FibLevel::standard(Tool::FibRetracement)
            .into_iter()
            .filter(|l| [0.382, 0.5, 0.618].contains(&l.value))
            .collect();
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

//! Small building blocks shared by the shell: toolbar buttons, dividers and
//! drag handles. Hover shading uses `t.hover_wash()` (a touch of the text
//! colour), which shows on every surface; `t.hover` alone nearly matches
//! cards in some themes.

use crate::motion;
use crate::theme::{self, Theme};
use egui::{Align2, Color32, CornerRadius, Rect, Response, Sense, Stroke, Ui, vec2};

/// A square icon button, as in the drawing toolbar and the widget rail.
/// `active` marks the selected tool or open widget.
pub fn tool_button(ui: &mut Ui, t: &Theme, glyph: &str, tip: &str, active: bool, size: f32) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(size, size), Sense::click());
    let hover = motion::toggle(ui.ctx(), resp.id, resp.hovered(), motion::MICRO);
    let on = motion::toggle(ui.ctx(), resp.id.with("on"), active, motion::MICRO);
    if ui.is_rect_visible(rect) {
        let p = ui.painter();
        let r = CornerRadius::same(7);
        p.rect_filled(rect, r, motion::with_alpha(t.hover_wash(), hover));
        p.rect_filled(rect, r, motion::with_alpha(t.accent_soft(), on));
        let color = motion::lerp_color(motion::lerp_color(t.text2, t.text, hover), t.accent, on);
        p.text(
            rect.center(),
            Align2::CENTER_CENTER,
            glyph,
            theme::regular(size * 0.56),
            color,
        );
    }
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    if tip.is_empty() { resp } else { resp.on_hover_text(tip) }
}

/// A text (optionally icon + text) button for the top bar. `active` shows
/// it as selected, like the current interval.
pub fn bar_button(ui: &mut Ui, t: &Theme, icon: Option<&str>, text: &str, tip: &str, active: bool) -> Response {
    let font = theme::medium(13.0);
    let label = match icon {
        Some(i) if text.is_empty() => i.to_string(),
        Some(i) => format!("{i}  {text}"),
        None => text.to_string(),
    };
    let galley = ui.painter().layout_no_wrap(label, font, t.text);
    let h = 30.0;
    let w = (galley.size().x + 16.0).max(h);
    let (rect, resp) = ui.allocate_exact_size(vec2(w, h), Sense::click());
    let hover = motion::toggle(ui.ctx(), resp.id, resp.hovered(), motion::MICRO);
    if ui.is_rect_visible(rect) {
        let p = ui.painter();
        p.rect_filled(rect, CornerRadius::same(7), motion::with_alpha(t.hover_wash(), hover));
        let color = if active {
            t.accent
        } else {
            motion::lerp_color(t.text2, t.text, hover.max(0.35))
        };
        p.galley_with_override_text_color(rect.center() - galley.size() / 2.0, galley, color);
    }
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    if tip.is_empty() { resp } else { resp.on_hover_text(tip) }
}

/// A square top-bar button showing a drawn icon (`paint` gets the icon's
/// rect and colour).
pub fn bar_icon_button(
    ui: &mut Ui,
    t: &Theme,
    tip: &str,
    paint: impl FnOnce(&egui::Painter, Rect, Color32),
) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(30.0, 30.0), Sense::click());
    let hover = motion::toggle(ui.ctx(), resp.id, resp.hovered(), motion::MICRO);
    if ui.is_rect_visible(rect) {
        let p = ui.painter();
        p.rect_filled(rect, CornerRadius::same(7), motion::with_alpha(t.hover_wash(), hover));
        let c = motion::lerp_color(t.text2, t.text, hover.max(0.35));
        paint(p, Rect::from_center_size(rect.center(), vec2(17.0, 17.0)), c);
    }
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    if tip.is_empty() { resp } else { resp.on_hover_text(tip) }
}

/// A thin vertical rule between groups in a horizontal bar.
pub fn vdivider(ui: &mut Ui, t: &Theme) {
    let (rect, _) = ui.allocate_exact_size(vec2(9.0, 22.0), Sense::hover());
    ui.painter()
        .vline(rect.center().x, rect.y_range(), Stroke::new(1.0, t.border));
}

/// A thin horizontal rule between groups in a vertical bar.
pub fn hdivider(ui: &mut Ui, t: &Theme, width: f32) {
    let (rect, _) = ui.allocate_exact_size(vec2(width, 9.0), Sense::hover());
    ui.painter()
        .hline(rect.x_range().shrink(6.0), rect.center().y, Stroke::new(1.0, t.border));
}

/// An invisible strip that can be dragged to resize a panel. Returns the
/// drag delta along its axis this frame (0 when idle) and paints a faint
/// accent line while hovered or dragged.
pub fn drag_handle(ui: &mut Ui, t: &Theme, rect: Rect, id: &str, vertical: bool) -> f32 {
    let resp = ui.interact(rect, ui.id().with(id), Sense::drag());
    let lit = motion::toggle(ui.ctx(), resp.id, resp.hovered() || resp.dragged(), motion::MICRO);
    // Always a faint grip in the middle, so it's clear where to drag; on
    // hover the grip takes the accent and a thin line marks the edge.
    let c = rect.center();
    let grip = if vertical {
        Rect::from_center_size(c, egui::vec2(4.0, 36.0))
    } else {
        Rect::from_center_size(c, egui::vec2(36.0, 4.0))
    };
    let p = ui.painter();
    if lit > 0.0 {
        let line = motion::with_alpha(t.accent, lit * 0.45);
        if vertical {
            p.vline(c.x, rect.y_range(), Stroke::new(1.0, line));
        } else {
            p.hline(rect.x_range(), c.y, Stroke::new(1.0, line));
        }
    }
    // Side handles sit in the gap between cards, where a resting grip reads
    // as a scrollbar: they show on hover only.
    let rest = if vertical {
        Color32::TRANSPARENT
    } else {
        t.text3.gamma_multiply(0.55)
    };
    p.rect_filled(grip, CornerRadius::same(2), motion::lerp_color(rest, t.accent, lit));
    let resp = resp.on_hover_cursor(if vertical {
        egui::CursorIcon::ResizeHorizontal
    } else {
        egui::CursorIcon::ResizeVertical
    });
    let d = resp.drag_delta();
    if vertical { d.x } else { d.y }
}

// ------------------------------------------------------------- form controls
//
// After Magpie's widgets. Colours come from the active visuals (accent =
// `selection.stroke`, edges = `widgets.inactive.bg_stroke`), so these drop
// into any code without a theme to hand.

fn accent(v: &egui::Visuals) -> Color32 {
    v.selection.stroke.color
}

/// A rounded checkbox: an accent square with a drawn tick when on.
/// Drop-in for `Ui::checkbox`.
pub fn checkbox(ui: &mut Ui, on: &mut bool, label: impl Into<egui::WidgetText>) -> Response {
    let v = ui.visuals().clone();
    let text = label.into();
    let galley = (!text.is_empty()).then(|| {
        text.into_galley(
            ui,
            Some(egui::TextWrapMode::Extend),
            f32::INFINITY,
            egui::TextStyle::Body,
        )
    });
    let box_s = 17.0;
    let w = box_s + galley.as_ref().map_or(0.0, |g| g.size().x + 8.0);
    let h = galley.as_ref().map_or(box_s, |g| g.size().y).max(24.0);
    let (rect, mut resp) = ui.allocate_exact_size(vec2(w, h), Sense::click());
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    let k = motion::toggle(ui.ctx(), resp.id.with("on"), *on, motion::MICRO);
    let hover = motion::toggle(ui.ctx(), resp.id.with("h"), resp.hovered(), motion::MICRO);
    if ui.is_rect_visible(rect) {
        let p = ui.painter();
        let b = Rect::from_center_size(
            egui::pos2(rect.left() + box_s / 2.0, rect.center().y),
            vec2(box_s, box_s),
        );
        let edge = motion::lerp_color(v.widgets.inactive.bg_stroke.color, v.text_color(), hover * 0.35);
        p.rect(
            b,
            CornerRadius::same(5),
            motion::lerp_color(v.extreme_bg_color, accent(&v), k),
            Stroke::new(1.0, motion::lerp_color(edge, accent(&v), k)),
            egui::StrokeKind::Inside,
        );
        if k > 0.01 {
            let c = motion::with_alpha(Color32::WHITE, k);
            let pts = [
                b.left_top() + vec2(4.4, 8.8),
                b.left_top() + vec2(7.2, 11.6),
                b.left_top() + vec2(12.8, 5.6),
            ];
            p.add(egui::Shape::line(pts.to_vec(), Stroke::new(1.8, c)));
        }
        if let Some(g) = galley {
            let pos = egui::pos2(b.right() + 8.0, rect.center().y - g.size().y / 2.0);
            p.galley(pos, g, v.text_color());
        }
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A round radio button. Drop-in for `Ui::radio_value`.
pub fn radio<V: PartialEq>(ui: &mut Ui, current: &mut V, value: V, label: impl Into<egui::WidgetText>) -> Response {
    let v = ui.visuals().clone();
    let galley = label.into().into_galley(
        ui,
        Some(egui::TextWrapMode::Extend),
        f32::INFINITY,
        egui::TextStyle::Body,
    );
    let d = 17.0;
    let size = vec2(d + 8.0 + galley.size().x, galley.size().y.max(24.0));
    let (rect, mut resp) = ui.allocate_exact_size(size, Sense::click());
    let mut sel = *current == value;
    if resp.clicked() && !sel {
        *current = value;
        sel = true;
        resp.mark_changed();
    }
    let k = motion::toggle(ui.ctx(), resp.id.with("on"), sel, motion::MICRO);
    let hover = motion::toggle(ui.ctx(), resp.id.with("h"), resp.hovered(), motion::MICRO);
    if ui.is_rect_visible(rect) {
        let p = ui.painter();
        let c = egui::pos2(rect.left() + d / 2.0, rect.center().y);
        let edge = motion::lerp_color(v.widgets.inactive.bg_stroke.color, v.text_color(), hover * 0.35);
        p.circle(
            c,
            d / 2.0 - 0.5,
            motion::lerp_color(v.extreme_bg_color, accent(&v), k),
            Stroke::new(1.0, motion::lerp_color(edge, accent(&v), k)),
        );
        if k > 0.01 {
            p.circle_filled(c, 3.2 * k, Color32::WHITE);
        }
        let pos = egui::pos2(rect.left() + d + 8.0, rect.center().y - galley.size().y / 2.0);
        p.galley(pos, galley, v.text_color());
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A segmented control (dialog tabs and small either/or choices) with a
/// sliding highlight. Returns true when the choice changed.
pub fn segmented<V: PartialEq + Copy>(ui: &mut Ui, t: &Theme, current: &mut V, options: &[(V, &str)]) -> bool {
    let id = ui.next_auto_id().with("seg");
    let font = theme::medium(12.5);
    let galleys: Vec<_> = options
        .iter()
        .map(|(_, o)| ui.painter().layout_no_wrap(o.to_string(), font.clone(), t.text))
        .collect();
    let widths: Vec<f32> = galleys.iter().map(|g| g.size().x + 24.0).collect();
    let total: f32 = widths.iter().sum::<f32>() + 6.0;
    let (rect, _) = ui.allocate_exact_size(vec2(total, 32.0), Sense::hover());
    let p = ui.painter().clone();
    let track = if t.dark { t.bg } else { t.sidebar };
    p.rect(
        rect,
        CornerRadius::same(9),
        track,
        Stroke::new(1.0, motion::with_alpha(t.border, 0.7)),
        egui::StrokeKind::Inside,
    );
    let mut changed = false;
    let mut x = rect.left() + 3.0;
    let mut rects = Vec::new();
    let mut hovered = None;
    for (i, w) in widths.iter().enumerate() {
        let r = Rect::from_min_size(egui::pos2(x, rect.top() + 3.0), vec2(*w, rect.height() - 6.0));
        let resp = ui
            .interact(r, id.with(i), Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        if resp.hovered() {
            hovered = Some(i);
        }
        if resp.clicked() && *current != options[i].0 {
            *current = options[i].0;
            changed = true;
        }
        rects.push(r);
        x += w;
    }
    let sel = options.iter().position(|(v, _)| v == current);
    if let Some(target) = sel.and_then(|s| rects.get(s)) {
        let l = motion::tween(ui.ctx(), id.with("l"), target.left(), motion::STANDARD);
        let r = motion::tween(ui.ctx(), id.with("r"), target.right(), motion::STANDARD);
        let hl = Rect::from_x_y_ranges(l..=r, target.y_range());
        p.rect_filled(
            hl.translate(vec2(0.0, 1.0)),
            CornerRadius::same(7),
            Color32::from_black_alpha(if t.dark { 50 } else { 14 }),
        );
        let fill = if t.dark {
            t.elevated.lerp_to_gamma(t.text, 0.10)
        } else {
            t.card
        };
        p.rect_filled(hl, CornerRadius::same(7), fill);
    }
    for (i, (g, r)) in galleys.into_iter().zip(&rects).enumerate() {
        let c = if Some(i) == sel {
            t.text
        } else if Some(i) == hovered {
            t.text.lerp_to_gamma(t.text2, 0.4)
        } else {
            t.text2
        };
        p.galley(r.center() - g.size() / 2.0, g, c);
    }
    changed
}

/// Button looks: a filled accent button for the main action, a quiet
/// bordered one for the rest, and a red-tinted one for deleting.
#[derive(Clone, Copy, PartialEq)]
pub enum Kind {
    Primary,
    Secondary,
    Danger,
}

pub fn button(ui: &mut Ui, kind: Kind, enabled: bool, icon: Option<&str>, label: &str) -> Response {
    let v = ui.visuals().clone();
    let font = theme::medium(13.0);
    let text = match icon {
        Some(i) if !label.is_empty() => format!("{i}  {label}"),
        Some(i) => i.to_string(),
        None => label.to_string(),
    };
    let galley = ui.painter().layout_no_wrap(text, font, v.text_color());
    let size = vec2((galley.size().x + 28.0).max(72.0), 32.0);
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, resp) = ui.allocate_exact_size(size, sense);
    let hover = motion::toggle(ui.ctx(), resp.id.with("h"), enabled && resp.hovered(), motion::MICRO);
    if ui.is_rect_visible(rect) {
        let p = ui.painter();
        let a = accent(&v);
        let neg = v.error_fg_color;
        let lift = if v.dark_mode { Color32::WHITE } else { Color32::BLACK };
        let (bg, stroke, fg) = match kind {
            Kind::Primary => (motion::lerp_color(a, lift, 0.08 * hover), Stroke::NONE, Color32::WHITE),
            Kind::Secondary => (
                motion::lerp_color(v.widgets.inactive.bg_fill, v.widgets.hovered.bg_fill, hover),
                Stroke::new(
                    1.0,
                    motion::lerp_color(v.widgets.inactive.bg_stroke.color, motion::with_alpha(a, 0.6), hover),
                ),
                v.text_color(),
            ),
            Kind::Danger => (
                motion::with_alpha(neg, 0.10 + 0.08 * hover),
                Stroke::new(1.0, motion::with_alpha(neg, 0.35)),
                neg,
            ),
        };
        let fade = if enabled { 1.0 } else { 0.45 };
        p.rect(
            rect,
            CornerRadius::same(8),
            motion::with_alpha(bg, fade * bg.a() as f32 / 255.0),
            stroke,
            egui::StrokeKind::Inside,
        );
        p.galley_with_override_text_color(
            rect.center() - galley.size() / 2.0,
            galley,
            motion::with_alpha(fg, fade),
        );
    }
    if enabled {
        resp.on_hover_cursor(egui::CursorIcon::PointingHand)
    } else {
        resp
    }
}

pub fn primary(ui: &mut Ui, label: &str) -> Response {
    button(ui, Kind::Primary, true, None, label)
}

pub fn secondary(ui: &mut Ui, label: &str) -> Response {
    button(ui, Kind::Secondary, true, None, label)
}

/// A single-line text field with room to breathe (egui's default margin
/// makes fields shorter than the dropdowns beside them).
pub fn field(text: &mut dyn egui::TextBuffer) -> egui::TextEdit<'_> {
    egui::TextEdit::singleline(text).margin(vec2(12.0, 8.0))
}

/// A search field with the magnifying glass drawn in its own gutter, so
/// the text cursor never sits on top of it.
pub fn search_field(ui: &mut Ui, t: &Theme, text: &mut String, hint: &str, width: f32) -> Response {
    let r = ui.add(
        egui::TextEdit::singleline(text)
            // A little air between the cursor and the hint.
            .hint_text(egui::RichText::new(format!(" {hint}")).color(t.text3))
            .desired_width(width)
            .margin(egui::Margin {
                left: 34,
                right: 10,
                top: 8,
                bottom: 8,
            }),
    );
    ui.painter().text(
        egui::pos2(r.rect.left() + 17.0, r.rect.center().y),
        Align2::CENTER_CENTER,
        crate::icons::ph::MAGNIFYING_GLASS,
        theme::regular(15.0),
        if r.has_focus() { t.text2 } else { t.text3 },
    );
    r
}

/// egui's combo box with a caret instead of the filled triangle. Drop-in
/// for `egui::ComboBox::from_id_salt`.
pub fn combo(id: impl std::hash::Hash + std::fmt::Debug) -> egui::ComboBox {
    egui::ComboBox::from_id_salt(id)
        .height(320.0)
        .icon(|ui, rect, wv, open| {
            ui.painter().text(
                rect.center(),
                Align2::CENTER_CENTER,
                if open {
                    crate::icons::ph::CARET_UP
                } else {
                    crate::icons::ph::CARET_DOWN
                },
                theme::regular(12.0),
                wv.fg_stroke.color,
            );
        })
}

/// Space kept clear on the right of menu rows for the floating scrollbar.
const MENU_GUTTER: f32 = 6.0;

/// A row in a dropdown or menu: soft hover, and for the current choice a
/// faint accent wash with a check mark. Drop-in for `Ui::selectable_label`.
pub fn option(ui: &mut Ui, selected: bool, text: impl Into<egui::WidgetText>) -> Response {
    let v = ui.visuals().clone();
    let a = accent(&v);
    let pad = 10.0;
    let check_w = if selected { 22.0 } else { 0.0 };
    let width = (ui.available_width() - MENU_GUTTER).max(60.0);
    let galley = text.into().into_galley(
        ui,
        Some(egui::TextWrapMode::Truncate),
        width - pad * 2.0 - check_w,
        egui::TextStyle::Body,
    );
    let height = (galley.size().y + 12.0).max(30.0);
    let (rect, resp) = ui.allocate_exact_size(vec2(width, height), Sense::click());
    if ui.is_rect_visible(rect) {
        let hover = motion::toggle(ui.ctx(), resp.id.with("hover"), resp.hovered(), motion::MICRO);
        let p = ui.painter();
        let radius = CornerRadius::same(7);
        if selected {
            let fill = motion::with_alpha(a, if v.dark_mode { 0.16 } else { 0.11 } + 0.05 * hover);
            p.rect_filled(rect, radius, fill);
            p.text(
                egui::pos2(rect.right() - pad, rect.center().y),
                Align2::RIGHT_CENTER,
                crate::icons::ph::CHECK,
                theme::semibold(13.0),
                a,
            );
        } else if hover > 0.0 {
            let wash = if v.dark_mode { 0.075 } else { 0.06 };
            p.rect_filled(rect, radius, motion::with_alpha(v.text_color(), wash * hover));
        }
        let pos = egui::pos2(rect.left() + pad, rect.center().y - galley.size().y / 2.0);
        p.galley(pos, galley, v.text_color());
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Drop-in for `Ui::selectable_value`, drawn with [`option`].
pub fn option_value<V: PartialEq>(
    ui: &mut Ui,
    current: &mut V,
    value: V,
    text: impl Into<egui::WidgetText>,
) -> Response {
    let mut resp = option(ui, *current == value, text);
    if resp.clicked() && *current != value {
        *current = value;
        resp.mark_changed();
    }
    resp
}

/// A menu row: icon, label, and an optional muted hint on the right (a
/// shortcut, a count). `selected` shows it in the accent colour.
pub fn menu_item(ui: &mut Ui, t: &Theme, icon: Option<&str>, label: &str, hint: &str, selected: bool) -> Response {
    menu_row(ui, t, icon, label, hint, selected, true)
}

pub fn menu_row(
    ui: &mut Ui,
    t: &Theme,
    icon: Option<&str>,
    label: &str,
    hint: &str,
    selected: bool,
    enabled: bool,
) -> Response {
    menu_row_tinted(ui, t, icon, label, hint, selected, enabled, None)
}

/// A menu row whose icon is drawn by `paint` (rect, colour).
pub fn menu_item_painted(
    ui: &mut Ui,
    t: &Theme,
    paint: impl FnOnce(&egui::Painter, Rect, Color32),
    label: &str,
    selected: bool,
) -> Response {
    let resp = menu_row_tinted(ui, t, Some(" "), label, "", selected, true, None);
    if ui.is_rect_visible(resp.rect) {
        let c = if selected { t.accent } else { t.text2 };
        let icon = Rect::from_center_size(
            egui::pos2(resp.rect.left() + 18.0, resp.rect.center().y),
            vec2(18.0, 18.0),
        );
        paint(ui.painter(), icon, c);
    }
    resp
}

/// A menu row in red, for removing or deleting.
pub fn menu_danger(ui: &mut Ui, t: &Theme, icon: Option<&str>, label: &str) -> Response {
    menu_row_tinted(ui, t, icon, label, "", false, true, Some(t.neg))
}

#[allow(clippy::too_many_arguments)]
fn menu_row_tinted(
    ui: &mut Ui,
    t: &Theme,
    icon: Option<&str>,
    label: &str,
    hint: &str,
    selected: bool,
    enabled: bool,
    tint: Option<Color32>,
) -> Response {
    let width = ui.available_width().max(120.0);
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, resp) = ui.allocate_exact_size(vec2(width, 32.0), sense);
    if ui.is_rect_visible(rect) {
        let hover = motion::toggle(
            ui.ctx(),
            resp.id.with("hover"),
            enabled && resp.hovered(),
            motion::MICRO,
        );
        let p = ui.painter();
        if selected {
            p.rect_filled(rect, CornerRadius::same(8), t.accent_soft());
        }
        p.rect_filled(rect, CornerRadius::same(8), motion::with_alpha(t.hover_wash(), hover));
        let base = if !enabled {
            t.text3
        } else if let Some(c) = tint {
            c
        } else if selected {
            t.accent
        } else {
            t.text
        };
        let mut x = rect.left() + 10.0;
        if let Some(i) = icon {
            p.text(
                egui::pos2(x + 8.0, rect.center().y),
                Align2::CENTER_CENTER,
                i,
                theme::regular(16.0),
                if selected || !enabled || tint.is_some() {
                    base
                } else {
                    t.text2
                },
            );
            x += 26.0;
        }
        p.text(
            egui::pos2(x, rect.center().y),
            Align2::LEFT_CENTER,
            label,
            theme::regular(13.5),
            base,
        );
        if !hint.is_empty() {
            p.text(
                egui::pos2(rect.right() - 10.0, rect.center().y),
                Align2::RIGHT_CENTER,
                hint,
                theme::regular(12.0),
                t.text3,
            );
        }
    }
    if enabled {
        resp.on_hover_cursor(egui::CursorIcon::PointingHand)
    } else {
        resp
    }
}

/// A small heading inside a menu.
pub fn menu_heading(ui: &mut Ui, t: &Theme, text: &str) {
    ui.add_space(2.0);
    ui.horizontal(|ui| {
        ui.add_space(10.0);
        ui.label(egui::RichText::new(text).font(theme::semibold(11.0)).color(t.text3));
    });
}

/// A hairline between groups in a menu.
pub fn menu_rule(ui: &mut Ui, t: &Theme) {
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 9.0), Sense::hover());
    ui.painter()
        .hline(r.x_range().shrink(4.0), r.center().y, Stroke::new(1.0, t.border));
}

/// A calm "nothing here yet" block: a large faint icon, a title and a line
/// of help.
pub fn empty_state(ui: &mut Ui, t: &Theme, glyph: &str, title: &str) {
    ui.vertical_centered(|ui| {
        ui.add_space(18.0);
        ui.label(egui::RichText::new(glyph).font(theme::regular(34.0)).color(t.text3));
        ui.add_space(4.0);
        ui.label(egui::RichText::new(title).font(theme::semibold(14.0)).color(t.text));
        ui.add_space(12.0);
    });
}

/// A solid five-point star (the icon font only has the outline), drawn as
/// a fan from its centre. `r` is the outer radius.
pub fn filled_star(p: &egui::Painter, c: egui::Pos2, r: f32, color: Color32) {
    use std::f32::consts::PI;
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(c, color);
    for k in 0..10 {
        let a = -PI / 2.0 + k as f32 * PI / 5.0;
        let rr = if k % 2 == 0 { r } else { r * 0.45 };
        mesh.colored_vertex(c + vec2(a.cos(), a.sin()) * rr, color);
    }
    for k in 0..10u32 {
        mesh.add_triangle(0, 1 + k, 1 + (k + 1) % 10);
    }
    p.add(mesh);
}

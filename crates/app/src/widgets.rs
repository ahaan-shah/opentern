//! Small building blocks shared by the shell: toolbar buttons, dividers and
//! drag handles. Hover shading uses `t.hover_wash()` (a touch of the text
//! colour), which shows on every surface; `t.hover` alone nearly matches
//! cards in some themes.

use crate::icons::ph;
use crate::motion;
use crate::theme::{self, Theme};
use egui::{Align2, Color32, CornerRadius, Rect, Response, Sense, Stroke, Ui, vec2};

/// A square icon button, as in the drawing toolbar and the widget rail.
/// `active` marks the selected tool or open widget.
pub fn tool_button(ui: &mut Ui, t: &Theme, glyph: &str, tip: &str, active: bool, size: f32) -> Response {
    let (_, resp) = ui.allocate_exact_size(vec2(size, size), Sense::click());
    tool_button_look(ui, t, resp, glyph, tip, active)
}

/// A `tool_button`'s look on a response the caller made (a button laid
/// out by hand, or sensing drags too).
pub fn tool_button_look(ui: &Ui, t: &Theme, resp: Response, glyph: &str, tip: &str, active: bool) -> Response {
    let rect = resp.rect;
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
            theme::regular(rect.height() * 0.56),
            color,
        );
    }
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    if tip.is_empty() { resp } else { resp.on_hover_text(tip) }
}

/// A text (optionally icon + text) button for the top bar. `active` shows
/// it as selected, like the current interval.
pub fn bar_button(ui: &mut Ui, t: &Theme, icon: Option<&str>, text: &str, tip: &str, active: bool) -> Response {
    // The icon at the toolbars' size (as `tool_button` draws it), the text
    // at the bar's.
    let mut job = egui::text::LayoutJob::default();
    let fmt = |font| egui::TextFormat {
        font_id: font,
        color: t.text,
        valign: egui::Align::Center,
        ..Default::default()
    };
    if let Some(i) = icon {
        job.append(i, 0.0, fmt(theme::regular(16.8)));
    }
    if !text.is_empty() {
        let gap = if icon.is_some() { 6.0 } else { 0.0 };
        job.append(text, gap, fmt(theme::medium(13.0)));
    }
    let galley = ui.fonts_mut(|f| f.layout_job(job));
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

/// The widest of `texts` in `font`, so a list can make room for longer
/// languages.
pub fn widest<'a>(ui: &Ui, font: egui::FontId, texts: impl IntoIterator<Item = &'a str>) -> f32 {
    texts
        .into_iter()
        .map(|s| {
            ui.painter()
                .layout_no_wrap(s.to_string(), font.clone(), Color32::WHITE)
                .size()
                .x
        })
        .fold(0.0, f32::max)
}

/// A row of tabs at most `room` wide that scrolls sideways when they don't
/// fit. `tabs` is told when to bring the open one into view: when it, the
/// language or the room changes (not every frame, so it can be scrolled away).
pub fn tab_scroll<K: Copy + PartialEq + Send + Sync + 'static>(
    ui: &mut Ui,
    id: &'static str,
    open: K,
    room: f32,
    tabs: impl FnOnce(&mut Ui, bool),
) {
    let seen_id = egui::Id::new((id, "seen"));
    let now = (open, crate::i18n::current() as u8, room as i32);
    let reveal = ui.data(|d| d.get_temp(seen_id)) != Some(now);
    ui.data_mut(|d| d.insert_temp(seen_id, now));
    ui.scope(|ui| {
        ui.style_mut().always_scroll_the_only_direction = true;
        egui::ScrollArea::horizontal()
            .id_salt(id)
            .max_width(room)
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
            .show(ui, |ui| tabs(ui, reveal));
    });
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
        Rect::from_center_size(c, egui::vec2(5.0, 44.0))
    } else {
        Rect::from_center_size(c, egui::vec2(44.0, 5.0))
    };
    let p = ui.painter();
    if lit > 0.0 {
        let line = motion::with_alpha(t.accent, lit * 0.75);
        if vertical {
            p.vline(c.x, rect.y_range(), Stroke::new(2.0, line));
        } else {
            p.hline(rect.x_range(), c.y, Stroke::new(2.0, line));
        }
    }
    // Side handles sit in the gap between cards, where a resting grip reads
    // as a scrollbar: they show on hover only.
    let rest = if vertical {
        Color32::TRANSPARENT
    } else {
        t.text3.gamma_multiply(0.55)
    };
    p.rect_filled(grip, CornerRadius::same(3), motion::lerp_color(rest, t.accent, lit));
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
    // The theme puts its accent here at full strength (the selection
    // stroke has it at 60%, for focus rings).
    v.hyperlink_color
}

/// Text on an accent fill: near-black on a bright accent, white on a deep
/// one, so a button's label always reads.
pub fn ink_on(fill: Color32) -> Color32 {
    let l = |c: u8| {
        let c = c as f32 / 255.0;
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    let lum = 0.2126 * l(fill.r()) + 0.7152 * l(fill.g()) + 0.0722 * l(fill.b());
    if lum > 0.36 {
        Color32::from_rgb(4, 21, 13)
    } else {
        Color32::WHITE
    }
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
            let c = motion::with_alpha(ink_on(accent(&v)), k);
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
        // A ring that turns the accent when picked, with an accent dot
        // growing in its middle (no solid disc with a dark dot).
        let a = accent(&v);
        let edge = motion::lerp_color(v.widgets.inactive.bg_stroke.color, v.text_color(), hover * 0.35);
        p.circle(
            c,
            d / 2.0 - 0.75,
            v.text_edit_bg_color(),
            Stroke::new(1.5, motion::lerp_color(edge, a, k)),
        );
        if k > 0.01 {
            p.circle_filled(c, 4.2 * k, a);
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
    let mut hovers = Vec::new();
    for (i, w) in widths.iter().enumerate() {
        let r = Rect::from_min_size(egui::pos2(x, rect.top() + 3.0), vec2(*w, rect.height() - 6.0));
        let resp = ui
            .interact(r, id.with(i), Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        hovers.push(motion::hover(&resp));
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
        } else {
            motion::lerp_color(t.text2, t.text, 0.6 * hovers[i])
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
            Kind::Primary => (motion::lerp_color(a, lift, 0.10 * hover), Stroke::NONE, ink_on(a)),
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

/// Decimal places worth showing for `x` (up to 4): 2.5 → 1, 0.25 → 2.
pub fn decimals_of(x: f64) -> usize {
    (0..4)
        .find(|&d| ((x * 10f64.powi(d as i32)).round() - x * 10f64.powi(d as i32)).abs() < 1e-9)
        .unwrap_or(4)
}

/// A number box. A click puts the cursor where it lands (nothing gets
/// selected), typing edits the number, and ↑/↓ add or take away `step`,
/// kept within `min..=max`. `decimals` is how many places it shows (0 for
/// whole numbers). True when the value changed.
#[allow(clippy::too_many_arguments)]
pub fn number(ui: &mut Ui, id: egui::Id, value: &mut f64, step: f64, min: f64, max: f64, decimals: usize) -> bool {
    let fmt = |v: f64| format!("{v:.decimals$}");
    let key = id.with("typed");
    let focused = ui.memory(|m| m.has_focus(id));
    // While it has the cursor, the text as typed (it may be half a number).
    let mut text = if focused {
        ui.data_mut(|d| d.get_temp::<String>(key))
            .unwrap_or_else(|| fmt(*value))
    } else {
        fmt(*value)
    };
    let mut changed = false;
    if focused {
        let (up, down) = ui.input_mut(|i| {
            (
                i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp),
                i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown),
            )
        });
        let by = if up {
            step
        } else if down {
            -step
        } else {
            0.0
        };
        if by != 0.0 {
            let base = text
                .trim()
                .parse::<f64>()
                .ok()
                .filter(|v| v.is_finite())
                .unwrap_or(*value);
            // Rounded to what it shows, so steps of 0.1 never drift.
            let places = decimals.max(decimals_of(step)) as i32;
            let k = 10f64.powi(places);
            let v = (((base + by) * k).round() / k).clamp(min, max);
            if v != *value {
                *value = v;
                changed = true;
            }
            text = fmt(v);
        }
    }
    let w = ui
        .painter()
        .layout_no_wrap(text.clone(), egui::FontId::proportional(14.0), Color32::WHITE)
        .size()
        .x;
    let resp = ui.add(field(&mut text).id(id).desired_width((w + 34.0).clamp(56.0, 160.0)));
    if resp.changed()
        && let Ok(v) = text.trim().parse::<f64>()
        && v.is_finite()
    {
        let v = if decimals == 0 { v.round() } else { v }.clamp(min, max);
        if v != *value {
            *value = v;
            changed = true;
        }
    }
    if resp.has_focus() {
        ui.data_mut(|d| d.insert_temp(key, text));
    } else {
        ui.data_mut(|d| d.remove::<String>(key));
    }
    changed
}

/// A multi-line text field with the same padding as [`field`], so its
/// text and hint sit where a single-line field's do.
pub fn area(text: &mut dyn egui::TextBuffer) -> egui::TextEdit<'_> {
    egui::TextEdit::multiline(text).margin(vec2(12.0, 8.0))
}

/// A field's name, above it: small, medium weight, muted.
pub fn field_label(ui: &mut Ui, t: &Theme, text: &str) {
    ui.add_space(2.0);
    ui.label(egui::RichText::new(text).font(theme::medium(12.5)).color(t.text2));
    ui.add_space(-2.0);
}

/// A labelled field in a stacked form: the name above, the control below,
/// `width` wide.
pub fn stacked(ui: &mut Ui, t: &Theme, label: &str, width: f32, control: impl FnOnce(&mut Ui)) {
    ui.allocate_ui_with_layout(vec2(width, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| {
        ui.set_width(width);
        field_label(ui, t, label);
        control(ui);
    });
}

/// One side of a [`pair`]: its label and what draws its control (given
/// the width it has).
pub type Half<'a> = (&'a str, &'a mut dyn FnMut(&mut Ui, f32));

/// Two stacked fields side by side, each half the row.
pub fn pair(ui: &mut Ui, t: &Theme, left: Half<'_>, right: Half<'_>) {
    let gap = 12.0;
    let w = ((ui.available_width() - gap) / 2.0).floor();
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = gap;
        stacked(ui, t, left.0, w, |ui| (left.1)(ui, w));
        stacked(ui, t, right.0, w, |ui| (right.1)(ui, w));
    });
}

/// A toggle chip: icon and label in a pill, filled with a wash of the
/// accent while on. For picking any of a few options (where to notify).
pub fn chip(ui: &mut Ui, t: &Theme, on: &mut bool, icon: &str, label: &str) -> Response {
    let font = theme::medium(13.0);
    let g = ui.painter().layout_no_wrap(format!("{icon}  {label}"), font, t.text);
    let size = vec2(g.size().x + 26.0, 32.0);
    let (rect, mut resp) = ui.allocate_exact_size(size, Sense::click());
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    let k = motion::toggle(ui.ctx(), resp.id.with("on"), *on, motion::MICRO);
    let hover = motion::toggle(ui.ctx(), resp.id.with("h"), resp.hovered(), motion::MICRO);
    if ui.is_rect_visible(rect) {
        let p = ui.painter();
        // Off: the field look. On: the rich accent fill, its edge, and
        // the label and icon in the accent's ink.
        let off_fill = motion::lerp_color(t.field(), t.hovered(t.field()), hover);
        let fill = motion::lerp_color(off_fill, t.accent_soft(), k);
        let edge = motion::lerp_color(t.edge(), t.accent_edge(), k);
        p.rect(
            rect,
            CornerRadius::same(8),
            fill,
            Stroke::new(1.0, edge),
            egui::StrokeKind::Inside,
        );
        let off = t.text2.lerp_to_gamma(t.text, hover * 0.5);
        let fg = motion::lerp_color(off, t.accent_ink(), k);
        p.galley_with_override_text_color(rect.center() - g.size() / 2.0, g, fg);
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A slider: a small solid handle on a slim rail that fills with the
/// accent, and the value as plain text beside it (egui's own had a large
/// hollow handle and a framed number box).
pub fn slider(ui: &mut Ui, t: &Theme, value: &mut i32, range: std::ops::RangeInclusive<i32>, suffix: &str) -> Response {
    ui.horizontal(|ui| {
        let (lo, hi) = (*range.start(), *range.end());
        let r = ui
            .scope(|ui| {
                ui.spacing_mut().interact_size.y = 16.0;
                // egui's own rail and handle stay unseen: its rail runs past
                // the handle at both ends.
                let w = &mut ui.visuals_mut().widgets;
                for s in [&mut w.inactive, &mut w.hovered, &mut w.active] {
                    s.bg_fill = Color32::TRANSPARENT;
                    s.fg_stroke = Stroke::NONE;
                    s.bg_stroke = Stroke::NONE;
                }
                ui.add(egui::Slider::new(value, range).show_value(false).trailing_fill(false))
            })
            .inner;
        // The handle: white with a soft ring, where egui puts its own.
        let rad = r.rect.height() / 2.5;
        let span = r.rect.x_range().shrink(rad);
        let k = if hi > lo {
            (*value - lo) as f32 / (hi - lo) as f32
        } else {
            0.0
        };
        let c = egui::pos2(span.min + k * (span.max - span.min), r.rect.center().y);
        let p = ui.painter();
        // The rail ends under the handle, whichever end it's at.
        let rail = |to: f32, col: Color32| {
            let r = Rect::from_x_y_ranges(span.min..=to, c.y - 2.0..=c.y + 2.0);
            p.rect_filled(r, CornerRadius::same(2), col);
        };
        rail(span.max, t.field().lerp_to_gamma(t.text, 0.12));
        rail(c.x, t.accent);
        p.circle_filled(c + vec2(0.0, 1.0), rad + 1.0, Color32::from_black_alpha(70));
        p.circle_filled(
            c,
            rad + motion::hover_if(&r, r.hovered() || r.dragged()),
            Color32::WHITE,
        );
        ui.add_space(6.0);
        ui.label(
            egui::RichText::new(format!("{value}{suffix}"))
                .font(theme::medium(13.0))
                .color(t.text),
        );
        r
    })
    .inner
}

/// Underlined text to click; green under the pointer.
pub fn text_link(ui: &mut Ui, t: &Theme, text: &str) -> Response {
    let font = theme::regular(13.0);
    let galley = ui.painter().layout_no_wrap(text.to_string(), font, t.text2);
    let (r, resp) = ui.allocate_exact_size(galley.size(), Sense::click());
    let lit = motion::toggle(ui.ctx(), resp.id, resp.hovered() || resp.has_focus(), motion::MICRO);
    let col = motion::lerp_color(t.text2, t.accent, lit);
    let p = ui.painter();
    p.galley_with_override_text_color(r.min, galley, col);
    p.hline(r.x_range(), r.bottom() - 0.5, Stroke::new(1.0, col));
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A hairline across the whole of a dialog's body, with air around it.
pub fn form_rule(ui: &mut Ui, t: &Theme) {
    ui.add_space(6.0);
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter()
        .hline(r.x_range(), r.center().y, Stroke::new(1.0, t.border));
    ui.add_space(6.0);
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

/// Runs `add` with egui's hover look eased in, for its own widgets, which
/// switch looks in one frame. `id` is the widget's.
fn easing(ui: &mut Ui, id: egui::Id, add: impl FnOnce(&mut Ui) -> Response) -> Response {
    let id = id.with("eased");
    let over = ui.data(|d| d.get_temp::<bool>(id)).unwrap_or(false);
    let k = motion::toggle(ui.ctx(), id, over, motion::MICRO);
    let before = ui.visuals().widgets.clone();
    let (a, b) = (&before.inactive, &before.hovered);
    let mix = |x: Stroke, y: Stroke| {
        Stroke::new(
            motion::lerp(x.width, y.width, k),
            motion::lerp_color(x.color, y.color, k),
        )
    };
    let mut now = *b;
    now.bg_fill = motion::lerp_color(a.bg_fill, b.bg_fill, k);
    now.weak_bg_fill = motion::lerp_color(a.weak_bg_fill, b.weak_bg_fill, k);
    now.bg_stroke = mix(a.bg_stroke, b.bg_stroke);
    now.fg_stroke = mix(a.fg_stroke, b.fg_stroke);
    ui.visuals_mut().widgets.inactive = now;
    ui.visuals_mut().widgets.hovered = now;
    let resp = add(ui);
    ui.visuals_mut().widgets = before;
    let hovered = resp.hovered();
    if hovered != over {
        ui.data_mut(|d| d.insert_temp(id, hovered));
        ui.ctx().request_repaint();
    }
    resp
}

/// One of egui's own widgets, with its hover eased in.
pub fn eased(ui: &mut Ui, widget: impl egui::Widget) -> Response {
    let id = ui.next_auto_id();
    easing(ui, id, |ui| ui.add(widget))
}

/// egui's combo box with a caret instead of the filled triangle. Stands in
/// for `egui::ComboBox::from_id_salt`.
pub fn combo(id: impl std::hash::Hash + std::fmt::Debug) -> Combo {
    let salt = egui::Id::new(id);
    let inner = egui::ComboBox::from_id_salt(salt)
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
        });
    Combo { inner, salt }
}

/// See [`combo`].
pub struct Combo {
    inner: egui::ComboBox,
    salt: egui::Id,
}

impl Combo {
    pub fn width(self, width: f32) -> Combo {
        Combo {
            inner: self.inner.width(width),
            ..self
        }
    }

    pub fn height(self, height: f32) -> Combo {
        Combo {
            inner: self.inner.height(height),
            ..self
        }
    }

    pub fn truncate(self) -> Combo {
        Combo {
            inner: self.inner.truncate(),
            ..self
        }
    }

    pub fn selected_text(self, text: impl Into<egui::WidgetText>) -> Combo {
        Combo {
            inner: self.inner.selected_text(text),
            ..self
        }
    }

    pub fn show_ui<R>(self, ui: &mut Ui, menu: impl FnOnce(&mut Ui) -> R) -> egui::InnerResponse<Option<R>> {
        let id = ui.make_persistent_id(self.salt);
        let mut inner = None;
        let response = easing(ui, id, |ui| {
            let r = self.inner.show_ui(ui, menu);
            inner = r.inner;
            r.response
        });
        egui::InnerResponse { inner, response }
    }
}

/// Space kept clear on the right of menu rows for the floating scrollbar.
const MENU_GUTTER: f32 = 6.0;

/// A row in a dropdown or menu: soft hover, and for the current choice a
/// faint accent wash with a check mark. Drop-in for `Ui::selectable_label`.
pub fn option(ui: &mut Ui, selected: bool, text: impl Into<egui::WidgetText>) -> Response {
    let v = ui.visuals().clone();
    let a = accent(&v);
    let pad = 10.0;
    // Room for the check on every row, so a menu is as wide as its longest
    // choice (a narrow button no longer cuts "Dashed" to "Das…").
    let check_w = 22.0;
    let text = text.into();
    let natural = text
        .clone()
        .into_galley(
            ui,
            Some(egui::TextWrapMode::Extend),
            f32::INFINITY,
            egui::TextStyle::Body,
        )
        .size()
        .x;
    let width = (ui.available_width() - MENU_GUTTER)
        .max(natural + pad * 2.0 + check_w)
        .max(60.0);
    let galley = text.into_galley(
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
            // The rich selected fill (`Theme::accent_soft`), from the
            // visuals: the accent steeped into the dark (or the white).
            let base = if v.dark_mode {
                v.extreme_bg_color.lerp_to_gamma(Color32::BLACK, 0.3)
            } else {
                Color32::WHITE
            };
            let fill = base.lerp_to_gamma(a, if v.dark_mode { 0.19 } else { 0.13 } + 0.04 * hover);
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
    menu_row_tinted(ui, t, icon, label, hint, selected, enabled, None, 0.0)
}

/// A menu row whose icon is drawn by `paint` (rect, colour).
pub fn menu_item_painted(
    ui: &mut Ui,
    t: &Theme,
    paint: impl FnOnce(&egui::Painter, Rect, Color32),
    label: &str,
    selected: bool,
) -> Response {
    let resp = menu_row_tinted(ui, t, Some(" "), label, "", selected, true, None, 0.0);
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

/// A menu row that leaves `trail` points free at its right end for the
/// caller's own button (a star), with the hint before it.
pub fn menu_item_trailing(
    ui: &mut Ui,
    t: &Theme,
    icon: Option<&str>,
    label: &str,
    hint: &str,
    selected: bool,
    trail: f32,
) -> Response {
    menu_row_tinted(ui, t, icon, label, hint, selected, true, None, trail)
}

/// How wide a menu needs to be for this row to show whole.
pub fn menu_row_width(ui: &Ui, icon: bool, label: &str, hint: &str, trail: f32) -> f32 {
    let w = |s: &str, f| ui.painter().layout_no_wrap(s.to_string(), f, Color32::WHITE).size().x;
    let hint_w = if hint.is_empty() {
        0.0
    } else {
        HINT_GAP + w(hint, theme::regular(12.0))
    };
    10.0 + if icon { 26.0 } else { 0.0 } + w(label, theme::regular(13.5)) + hint_w + 10.0 + trail
}

/// The least room between a menu row's label and its hint.
const HINT_GAP: f32 = 16.0;

/// A menu row in red, for removing or deleting.
pub fn menu_danger(ui: &mut Ui, t: &Theme, icon: Option<&str>, label: &str) -> Response {
    menu_row_tinted(ui, t, icon, label, "", false, true, Some(t.neg), 0.0)
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
    trail: f32,
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
        // The hint first: the label gets what's left, and is shortened
        // with "…" rather than run under it (a long name, a long language).
        let mut end = rect.right() - 10.0 - trail;
        if !hint.is_empty() {
            let r = p.text(
                egui::pos2(end, rect.center().y),
                Align2::RIGHT_CENTER,
                hint,
                theme::regular(12.0),
                t.text3,
            );
            end = r.left() - HINT_GAP;
        }
        let mut job = egui::text::LayoutJob::simple_singleline(label.to_string(), theme::regular(13.5), base);
        job.wrap = egui::text::TextWrapping {
            max_width: (end - x).max(0.0),
            max_rows: 1,
            break_anywhere: true,
            overflow_character: Some('…'),
        };
        let g = ui.painter().layout_job(job);
        p.galley(egui::pos2(x, rect.center().y - g.size().y / 2.0), g, base);
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

/// A rounded progress bar whose fill eases towards `frac` (from Magpie's
/// `widgets::paint_progress`).
pub fn progress(ui: &mut Ui, t: &Theme, id: egui::Id, size: egui::Vec2, frac: f32) -> Response {
    let (rect, resp) = ui.allocate_exact_size(size, Sense::hover());
    let f = motion::tween_from(ui.ctx(), id, 0.0, frac.clamp(0.0, 1.0), motion::CHART);
    let p = ui.painter();
    let r = CornerRadius::same((rect.height() / 2.0) as u8);
    p.rect_filled(rect, r, t.hover_wash());
    if f > 0.001 {
        let w = (rect.width() * f).max(rect.height());
        p.rect_filled(Rect::from_min_size(rect.min, vec2(w, rect.height())), r, t.accent);
    }
    resp
}

/// A solid star: the outline star's filled twin from the icon font, so it
/// is smooth at any size and lines up with `ph::STAR`. `r` is about the
/// outer radius (6.5 matches a 14-point outline star).
pub fn filled_star(p: &egui::Painter, c: egui::Pos2, r: f32, color: Color32) {
    p.text(
        c,
        Align2::CENTER_CENTER,
        crate::icons::ph::STAR,
        theme::filled(r * 2.2),
        color,
    );
}

/// An icon button like `tool_button` whose icon turns while
/// `spinning`.
pub fn spin_button(ui: &mut Ui, t: &Theme, glyph: &str, tip: &str, spinning: bool) -> egui::Response {
    let size = 28.0;
    let (rect, resp) = ui.allocate_exact_size(vec2(size, size), Sense::click());
    let hover = motion::toggle(ui.ctx(), resp.id, resp.hovered() && !spinning, motion::MICRO);
    let p = ui.painter();
    p.rect_filled(rect, 7, motion::with_alpha(t.hover_wash(), hover));
    let color = if spinning {
        t.accent
    } else {
        motion::lerp_color(t.text2, t.text, hover)
    };
    let galley = p.layout_no_wrap(glyph.to_string(), theme::regular(size * 0.56), color);
    let angle = if spinning {
        ui.ctx().request_repaint();
        (ui.input(|i| i.time) * 5.0) as f32
    } else {
        0.0
    };
    // Turn about the glyph's centre.
    let half = galley.size() / 2.0;
    let pos = rect.center() - egui::emath::Rot2::from_angle(angle) * half;
    p.add(egui::epaint::TextShape::new(pos, galley, color).with_angle(angle));
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    resp.on_hover_text(tip)
}

/// A tick `size` wide, drawn up to `t` (0..1) along its stroke.
pub fn tick(p: &egui::Painter, c: egui::Pos2, size: f32, t: f32, color: Color32) {
    if t <= 0.0 {
        return;
    }
    let a = c + vec2(-0.46, 0.02) * size;
    let b = c + vec2(-0.14, 0.34) * size;
    let e = c + vec2(0.46, -0.32) * size;
    let (first, second) = ((b - a).length(), (e - b).length());
    let reach = (first + second) * t;
    let mut points = vec![a];
    if reach <= first {
        points.push(a + (b - a) * (reach / first));
    } else {
        points.push(b);
        points.push(b + (e - b) * ((reach - first) / second));
    }
    p.add(egui::Shape::line(points, Stroke::new(size * 0.135, color)));
}

/// How long a copy button stays a check.
const COPIED_FOR: f64 = 1.6;

/// A copy button in `rect`: when pressed, the two squares fade as a check
/// draws itself in, then it turns back. `id` holds when it was pressed.
pub fn copy_button(ui: &mut Ui, t: &Theme, text: &str, rect: Rect, id: egui::Id, icon: f32) {
    let resp = ui
        .interact(rect, id.with("copy"), Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    let now = ui.input(|i| i.time);
    if resp.clicked() {
        ui.ctx().copy_text(text.to_string());
        ui.ctx().data_mut(|d| d.insert_temp(id, now));
    }
    let at = ui.ctx().data(|d| d.get_temp::<f64>(id));
    let copied = at.is_some_and(|a| now - a < COPIED_FOR);
    if let Some(a) = at.filter(|_| copied) {
        let left = (COPIED_FOR - (now - a)).clamp(0.0, COPIED_FOR);
        ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(left));
    }
    let k = motion::toggle(ui.ctx(), resp.id.with("copied"), copied, motion::STANDARD);
    let hover = motion::toggle(ui.ctx(), resp.id.with("h"), resp.hovered(), motion::MICRO);

    let p = ui.painter();
    p.rect_filled(
        resp.rect,
        CornerRadius::same(7),
        motion::with_alpha(t.hover_wash(), hover * (1.0 - k)),
    );
    if k < 0.99 {
        let c = motion::lerp_color(t.text2, t.text, hover);
        p.text(
            rect.center(),
            Align2::CENTER_CENTER,
            ph::COPY,
            theme::regular(icon),
            motion::with_alpha(c, 1.0 - k),
        );
    }
    tick(p, rect.center(), icon * 0.9, k, t.accent);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::THEMES;

    fn shapes(mut f: impl FnMut(&mut Ui), events: Vec<egui::Event>, ctx: &egui::Context) -> Vec<egui::Shape> {
        let raw = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(400.0, 200.0))),
            events,
            ..Default::default()
        };
        let mut out = ctx.run_ui(raw, |ui| f(ui));
        out.textures_delta.clear();
        out.shapes.into_iter().map(|s| s.shape).collect()
    }

    /// At either end, and between, the handle covers the end of the green.
    #[test]
    fn the_slider_handle_covers_the_end_of_its_rail() {
        let ctx = egui::Context::default();
        theme::install_fonts(&ctx, "Inter");
        for v in [0, 1, 60, 99, 100] {
            let mut value = v;
            let all = shapes(
                |ui| {
                    slider(ui, &THEMES[0], &mut value, 0..=100, "%");
                },
                vec![],
                &ctx,
            );
            let (mut handle, mut green, mut widest) = (None, None, f32::MIN);
            for s in &all {
                match s {
                    egui::Shape::Circle(c) if c.fill == Color32::WHITE => handle = Some((c.center.x, c.radius)),
                    egui::Shape::Rect(r) if r.fill != Color32::TRANSPARENT && r.rect.height() < 6.0 => {
                        widest = widest.max(r.rect.right());
                        if r.fill == THEMES[0].accent {
                            green = Some(r.rect.right());
                        }
                    }
                    _ => {}
                }
            }
            let (x, r) = handle.expect("a handle");
            if v > 0 {
                assert!(green.is_some_and(|g| (g - x).abs() <= r), "{v}: {green:?} past {x}±{r}");
            }
            if v == 100 {
                assert!(widest <= x + r, "{widest} past {}", x + r);
            }
        }
    }

    /// Plain underlined text, green while the pointer is on it.
    #[test]
    fn a_text_link_lights_up() {
        let ctx = egui::Context::default();
        theme::install_fonts(&ctx, "Inter");
        let t = THEMES[0];
        let mut rect = Rect::NOTHING;
        let mut line = |events| {
            let mut col = Color32::TRANSPARENT;
            for _ in 0..40 {
                let all = shapes(|ui| rect = text_link(ui, &t, "Theme").rect, Vec::clone(&events), &ctx);
                for s in all {
                    if let egui::Shape::LineSegment { stroke, .. } = s {
                        col = stroke.color;
                    }
                }
            }
            (col, rect.center())
        };
        let (col, center) = line(vec![]);
        assert_eq!(col, t.text2);
        assert_eq!(line(vec![egui::Event::PointerMoved(center)]).0, t.accent);
    }

    /// A click puts the cursor in without selecting the number; ↑/↓ step
    /// it by the script's step, within its limits; typing sets it.
    #[test]
    fn number_box_steps_with_arrows_and_selects_nothing_on_click() {
        let ctx = egui::Context::default();
        theme::install_fonts(&ctx, "Inter");
        let id = egui::Id::new("n");
        let mut v = 2.5;
        let mut rect = Rect::NOTHING;
        let frame = |events: Vec<egui::Event>, v: &mut f64, rect: &mut Rect| {
            let raw = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(400.0, 200.0))),
                events,
                ..Default::default()
            };
            let mut out = ctx.run_ui(raw, |ui| {
                number(ui, id, v, 0.1, 0.0, 2.7, 1);
                *rect = ui.min_rect();
            });
            // No renderer here: drop the font atlas's upload.
            out.textures_delta.clear();
        };
        frame(vec![], &mut v, &mut rect);
        let at = rect.left_center() + vec2(14.0, 0.0);
        let button = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        frame(vec![egui::Event::PointerMoved(at)], &mut v, &mut rect);
        frame(vec![button(true)], &mut v, &mut rect);
        frame(vec![button(false)], &mut v, &mut rect);
        frame(vec![], &mut v, &mut rect);
        assert!(ctx.memory(|m| m.has_focus(id)), "the click focused it");
        let state = egui::TextEdit::load_state(&ctx, id).unwrap();
        let range = state.cursor.char_range().unwrap();
        assert_eq!(range.primary, range.secondary, "nothing selected");
        let key = |k| egui::Event::Key {
            key: k,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        frame(vec![key(egui::Key::ArrowUp)], &mut v, &mut rect);
        assert!((v - 2.6).abs() < 1e-12, "{v}");
        frame(vec![key(egui::Key::ArrowUp)], &mut v, &mut rect);
        frame(vec![key(egui::Key::ArrowUp)], &mut v, &mut rect);
        assert_eq!(v, 2.7, "kept within the maximum");
        frame(vec![key(egui::Key::ArrowDown)], &mut v, &mut rect);
        assert!((v - 2.6).abs() < 1e-12, "{v}");
        assert_eq!(decimals_of(0.25), 2);
        assert_eq!(decimals_of(3.0), 0);
    }
}

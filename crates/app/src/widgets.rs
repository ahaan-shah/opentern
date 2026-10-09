//! Small building blocks shared by the shell: toolbar buttons, dividers and
//! drag handles. Hover shading uses `t.hover_wash()` (a touch of the text
//! colour), which shows on every surface; `t.hover` alone nearly matches
//! cards in some themes.

use crate::motion;
use crate::theme::{self, Theme};
use egui::{Align2, CornerRadius, Rect, Response, Sense, Stroke, Ui, vec2};

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
    if lit > 0.0 {
        let c = motion::with_alpha(t.accent, lit * 0.8);
        if vertical {
            ui.painter().vline(rect.center().x, rect.y_range(), Stroke::new(2.0, c));
        } else {
            ui.painter().hline(rect.x_range(), rect.center().y, Stroke::new(2.0, c));
        }
    }
    let resp = resp.on_hover_cursor(if vertical {
        egui::CursorIcon::ResizeHorizontal
    } else {
        egui::CursorIcon::ResizeVertical
    });
    let d = resp.drag_delta();
    if vertical { d.x } else { d.y }
}

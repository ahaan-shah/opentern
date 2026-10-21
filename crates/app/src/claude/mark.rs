//! The panel's marks. The agents' face (the rail button, the agent picker)
//! is drawn by hand. The Claude mark is rendered once from
//! `assets/claude.svg` into a white mask, then tinted and rotated for the
//! spinner.
//!
//! The glyph is Simple Icons' "Claude" (CC0-1.0,
//! https://simpleicons.org). Claude is a trademark of Anthropic; it's used
//! here only to name the integration with the user's own Claude Code.

use crate::motion;
use crate::theme::Theme;
use egui::{Color32, CornerRadius, Rect, Response, Sense, TextureHandle, Ui, vec2};
use resvg::{tiny_skia, usvg};

/// Claude's brand colour.
pub const CLAUDE: Color32 = Color32::from_rgb(0xD9, 0x77, 0x57);

const SVG: &str = include_str!("../../assets/claude.svg");
const PX: u32 = 128;

fn texture(ctx: &egui::Context) -> Option<TextureHandle> {
    let id = egui::Id::new("claude-mark");
    if let Some(t) = ctx.data(|d| d.get_temp::<TextureHandle>(id)) {
        return Some(t);
    }
    let tree = usvg::Tree::from_str(SVG, &usvg::Options::default()).ok()?;
    let mut pm = tiny_skia::Pixmap::new(PX, PX)?;
    let k = PX as f32 / tree.size().width().max(tree.size().height());
    resvg::render(&tree, tiny_skia::Transform::from_scale(k, k), &mut pm.as_mut());
    let img = egui::ColorImage::from_rgba_premultiplied([PX as usize, PX as usize], pm.data());
    let tex = ctx.load_texture("claude-mark", img, egui::TextureOptions::LINEAR);
    ctx.data_mut(|d| d.insert_temp(id, tex.clone()));
    Some(tex)
}

/// Paints the mark in `rect`, tinted, turned by `angle` radians.
pub fn paint(ui: &Ui, rect: Rect, color: Color32, angle: f32) {
    let Some(tex) = texture(ui.ctx()) else { return };
    egui::Image::new(&tex)
        .tint(color)
        .rotate(angle, vec2(0.5, 0.5))
        .paint_at(ui, rect);
}

/// Paints the mark upright on any painter.
pub fn paint_flat(p: &egui::Painter, rect: Rect, color: Color32) {
    let Some(tex) = texture(p.ctx()) else { return };
    p.image(
        tex.id(),
        rect,
        Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
        color,
    );
}

/// The agents' face: a disc in the theme's colours with two pill-shaped eyes
/// (the panel's own mark, whichever agent is picked). `blink` 0–1 closes
/// the eyes.
pub fn face(ui: &Ui, center: egui::Pos2, radius: f32, t: &Theme, alpha: f32, blink: f32) {
    let p = ui.painter();
    // The theme's own colours: its text tinted a touch toward its accent,
    // with eyes in its background.
    let disc = t.text.lerp_to_gamma(t.accent, 0.22);
    let eye = t.bg;
    p.circle_filled(center, radius, motion::with_alpha(disc, alpha));
    // Big, tall, rounded eyes; closing squashes them.
    let open = 1.0 - 0.88 * blink.clamp(0.0, 1.0);
    let (w, h) = (radius * 0.3, (radius * 0.66 * open).max(radius * 0.1));
    for dx in [-0.34, 0.34] {
        let c = center + vec2(radius * dx, -radius * 0.02);
        let eye_r = Rect::from_center_size(c, vec2(w, h));
        p.rect_filled(
            eye_r,
            CornerRadius::same((w / 2.0).round() as u8),
            motion::with_alpha(eye, alpha),
        );
    }
}

/// The rail button: like `widgets::tool_button`, with the face for a
/// glyph. It blinks as the pointer arrives and leaves.
pub fn rail_button(ui: &mut Ui, t: &Theme, tip: &str, active: bool, size: f32) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(size, size), Sense::click());
    let hover = motion::toggle(ui.ctx(), resp.id, resp.hovered(), motion::STANDARD);
    let on = motion::toggle(ui.ctx(), resp.id.with("on"), active, motion::MICRO);
    if ui.is_rect_visible(rect) {
        let p = ui.painter();
        let r = CornerRadius::same(7);
        p.rect_filled(rect, r, motion::with_alpha(t.hover_wash(), hover.max(on)));
        let blink = (hover * std::f32::consts::PI).sin();
        face(ui, rect.center(), size * 0.29, t, 0.82 + 0.18 * hover.max(on), blink);
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(tip)
}

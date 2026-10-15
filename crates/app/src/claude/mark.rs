//! The Claude mark, for the panel that talks to Claude Code: rendered once
//! from `assets/claude.svg` into a white mask, then tinted (grey on the
//! rail, Claude's terracotta in the panel) and rotated for the spinner.
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

/// The mark as a widget of `size` points.
pub fn show(ui: &mut Ui, size: f32, color: Color32) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(size, size), Sense::hover());
    paint(ui, rect, color, 0.0);
    resp
}

/// The spinner while Claude works: the mark, turning.
pub fn spinner(ui: &mut Ui, size: f32) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(size, size), Sense::hover());
    let t = ui.input(|i| i.time) as f32;
    paint(ui, rect, CLAUDE, t * 2.2);
    ui.ctx().request_repaint();
    resp
}

/// The rail button: like `widgets::tool_button`, with the mark for a glyph.
pub fn rail_button(ui: &mut Ui, t: &Theme, tip: &str, active: bool, size: f32) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(size, size), Sense::click());
    let hover = motion::toggle(ui.ctx(), resp.id, resp.hovered(), motion::MICRO);
    let on = motion::toggle(ui.ctx(), resp.id.with("on"), active, motion::MICRO);
    if ui.is_rect_visible(rect) {
        let p = ui.painter();
        let r = CornerRadius::same(7);
        p.rect_filled(rect, r, motion::with_alpha(t.hover_wash(), hover));
        p.rect_filled(rect, r, motion::with_alpha(CLAUDE.gamma_multiply(0.16), on));
        let color = motion::lerp_color(motion::lerp_color(t.text2, t.text, hover), CLAUDE, on);
        paint(
            ui,
            Rect::from_center_size(rect.center(), vec2(size * 0.52, size * 0.52)),
            color,
            0.0,
        );
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(tip)
}

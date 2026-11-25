//! The agent panel's look: borderless cards with generous rounding, round
//! buttons, pills and plain meters, in the open agent's own colours (its
//! panel gets a theme whose accent is the agent's: `Agent::theme`).

use crate::motion;
use crate::theme::{self, Theme};
use egui::{Align2, Color32, Painter, Rect, Response, Sense, Stroke, Ui, pos2, vec2};

/// A card's rounding.
pub const CARD: u8 = 18;

/// A highlight's, on rows and buttons.
pub const RADIUS: u8 = 7;

/// What cards, bubbles and the composer are filled with.
pub fn surface(t: &Theme) -> Color32 {
    if t.dark { t.elevated } else { t.card }
}

/// A step up from `surface`: round buttons and pills on cards.
pub fn raised(t: &Theme) -> Color32 {
    if t.dark {
        t.hovered(t.elevated)
    } else {
        t.hovered(t.card)
    }
}

/// The accent, thin enough to sit behind text.
pub fn tint(t: &Theme) -> Color32 {
    motion::with_alpha(t.accent, if t.dark { 0.15 } else { 0.12 })
}

/// What's left of a meter or slider.
pub fn rail(t: &Theme) -> Color32 {
    motion::with_alpha(t.text3, if t.dark { 0.28 } else { 0.25 })
}

/// A meter: filled to `frac`, with an optional mark.
pub fn meter(p: &Painter, t: &Theme, r: Rect, frac: f32, fill: Color32, mark: Option<f32>, behind: Color32) {
    let frac = frac.clamp(0.0, 1.0);
    let round = r.height() / 2.0;
    p.rect_filled(r, round, rail(t));
    if frac > 0.0 {
        // Even a sliver is a whole dot.
        let x = (r.left() + r.width() * frac).max(r.left() + r.height());
        p.rect_filled(Rect::from_min_max(r.min, pos2(x, r.bottom())), round, fill);
    }
    if let Some(m) = mark {
        marker(p, r, r.left() + r.width() * m.clamp(0.0, 1.0), t.text, behind);
    }
}

/// An upright mark on a meter, with a little room cut out around it.
pub fn marker(p: &Painter, r: Rect, x: f32, color: Color32, behind: Color32) {
    let x = x.clamp(r.left() + 2.0, r.right() - 2.0);
    let c = pos2(x, r.center().y);
    p.rect_filled(Rect::from_center_size(c, vec2(8.0, r.height() + 2.0)), 0, behind);
    p.rect_filled(Rect::from_center_size(c, vec2(3.0, r.height() + 7.0)), 1.5, color);
}

/// An icon button, highlighted like the app's other ones.
pub fn icon_button(ui: &mut Ui, t: &Theme, glyph: &str, size: f32, icon: f32) -> Response {
    let (r, resp, hover) = button_base(ui, t, size);
    ui.painter().text(
        r.center(),
        Align2::CENTER_CENTER,
        glyph,
        theme::regular(icon),
        motion::lerp_color(t.text2, t.text, hover),
    );
    resp
}

fn button_base(ui: &mut Ui, t: &Theme, size: f32) -> (Rect, Response, f32) {
    let (r, resp) = ui.allocate_exact_size(vec2(size, size), Sense::click());
    let open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&resp));
    let hover = motion::hover_if(&resp, resp.hovered() || open);
    ui.painter()
        .rect_filled(r, RADIUS, motion::with_alpha(t.hover_wash(), hover));
    (r, resp.on_hover_cursor(egui::CursorIcon::PointingHand), hover)
}

/// The usage button: a ring filled as far as `used` (0–1), in `fill`.
pub fn usage_button(ui: &mut Ui, t: &Theme, used: Option<f64>, fill: Color32, size: f32) -> Response {
    let (r, resp, hover) = button_base(ui, t, size);
    let c = r.center();
    let rad = 7.0;
    let p = ui.painter();
    let track = motion::with_alpha(motion::lerp_color(t.text2, t.text, hover), 0.38);
    p.circle_stroke(c, rad, Stroke::new(2.4, track));
    if let Some(used) = used.filter(|u| *u > 0.0) {
        let end = used.clamp(0.04, 1.0) as f32 * std::f32::consts::TAU;
        let pts = (0..=48)
            .map(|k| c + egui::Vec2::angled(end * k as f32 / 48.0 - std::f32::consts::FRAC_PI_2) * rad)
            .collect();
        p.add(egui::Shape::line(pts, Stroke::new(2.4, fill)));
    }
    resp
}

/// An icon on a solid accent disc.
pub fn badge(p: &Painter, t: &Theme, c: egui::Pos2, radius: f32, glyph: &str, alpha: f32) {
    p.circle_filled(c, radius, motion::with_alpha(t.accent, alpha));
    p.text(
        c,
        Align2::CENTER_CENTER,
        glyph,
        theme::regular(radius * 1.08),
        motion::with_alpha(t.on_accent, alpha),
    );
}

/// A small label in a pill, right-aligned at `right`. Returns its left edge.
pub fn chip(p: &Painter, text: &str, right: egui::Pos2, fill: Color32, color: Color32) -> f32 {
    let g = p.layout_no_wrap(text.to_string(), theme::medium(11.0), color);
    let size = g.size() + vec2(16.0, 6.0);
    let r = Rect::from_min_size(pos2(right.x - size.x, right.y - size.y / 2.0), size);
    p.rect_filled(r, RADIUS, fill);
    p.galley(r.center() - g.size() / 2.0, g, color);
    r.left()
}

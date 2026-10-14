//! The colour picker: TradingView's swatch grid (greys, then ten hues in
//! seven shades), recent colours, a custom colour and an opacity slider.

use crate::chart::style::{c32, rgba};
use crate::motion;
use crate::theme::{self, Theme};
use egui::{Color32, CornerRadius, Id, Sense, Stroke, Ui, vec2};
use opentern_series::Rgba;

const GREYS: [u32; 10] = [
    0xFFFFFF, 0xD1D4DC, 0xB2B5BE, 0x9598A1, 0x787B86, 0x5D606B, 0x434651, 0x2A2E39, 0x131722, 0x000000,
];
const HUES: [u32; 10] = [
    0xF23645, 0xFF9800, 0xFFEB3B, 0x4CAF50, 0x089981, 0x00BCD4, 0x2962FF, 0x673AB7, 0x9C27B0, 0xE91E63,
];

fn mix(a: u32, b: u32, t: f32) -> u32 {
    let ch = |v: u32, s: u32| (((v >> s) & 0xFF) as f32, ((b >> s) & 0xFF) as f32);
    let mut out = 0;
    for s in [16, 8, 0] {
        let (x, y) = ch(a, s);
        out |= ((x + (y - x) * t).round() as u32) << s;
    }
    out
}

/// The swatch grid, row by row.
fn grid() -> Vec<Vec<u32>> {
    let mut rows = vec![GREYS.to_vec(), HUES.to_vec()];
    for t in [0.8, 0.6, 0.4, 0.2] {
        rows.push(HUES.iter().map(|h| mix(*h, 0xFFFFFF, t)).collect());
    }
    for t in [0.25, 0.5] {
        rows.push(HUES.iter().map(|h| mix(*h, 0x000000, t)).collect());
    }
    rows
}

/// A light checkerboard behind see-through colours. Kept inside the
/// rounded swatch: inset by the corner's overshoot.
fn checker(ui: &Ui, r: egui::Rect) {
    let p = ui.painter().with_clip_rect(r.intersect(ui.clip_rect()));
    p.rect_filled(r, CornerRadius::ZERO, Color32::from_gray(236));
    let s = 4.0;
    let (nx, ny) = ((r.width() / s).ceil() as i32, (r.height() / s).ceil() as i32);
    for i in 0..nx {
        for j in 0..ny {
            if (i + j) % 2 == 0 {
                let c = egui::Rect::from_min_size(r.min + vec2(i as f32 * s, j as f32 * s), vec2(s, s));
                p.rect_filled(c, CornerRadius::ZERO, Color32::from_gray(204));
            }
        }
    }
}

/// One swatch: the colour in a rounded square, a checkerboard showing
/// through only when it's see-through, and a faint edge so dark colours
/// still read on a dark dialog.
fn swatch(ui: &Ui, t: &Theme, r: egui::Rect, color: Rgba, radius: u8) {
    if color.a() < 255 {
        checker(ui, r.shrink(radius as f32 * 0.3));
    }
    let p = ui.painter();
    p.rect_filled(r, CornerRadius::same(radius), c32(color));
    p.rect_stroke(
        r,
        CornerRadius::same(radius),
        Stroke::new(1.0, motion::with_alpha(t.text, 0.14)),
        egui::StrokeKind::Inside,
    );
}

/// A colour swatch button that opens the picker. Returns true when the
/// colour changed.
pub fn color_button(ui: &mut Ui, t: &Theme, color: &mut Rgba) -> bool {
    let (rect, resp) = ui.allocate_exact_size(vec2(26.0, 26.0), Sense::click());
    let hov = motion::toggle(ui.ctx(), resp.id, resp.hovered(), motion::MICRO);
    let open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&resp));
    let inner = rect.shrink(2.0);
    swatch(ui, t, inner, *color, 6);
    let ring = if open { 1.0 } else { hov };
    if ring > 0.0 {
        ui.painter().rect_stroke(
            rect,
            CornerRadius::same(8),
            Stroke::new(1.5, motion::with_alpha(t.accent, ring)),
            egui::StrokeKind::Inside,
        );
    }
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    let mut changed = false;
    egui::Popup::from_toggle_button_response(&resp)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            changed = picker(ui, t, color);
        });
    changed
}

fn picker(ui: &mut Ui, t: &Theme, color: &mut Rgba) -> bool {
    let mut changed = false;
    let alpha = color.a();
    ui.spacing_mut().item_spacing = vec2(3.0, 3.0);
    for row in grid() {
        ui.horizontal(|ui| {
            for rgb in row {
                let (r, resp) = ui.allocate_exact_size(vec2(20.0, 20.0), Sense::click());
                let selected = (color.0 >> 8) == rgb;
                swatch(ui, t, r, Rgba::hex(rgb), 4);
                if selected || resp.hovered() {
                    ui.painter().rect_stroke(
                        r.expand(1.5),
                        CornerRadius::same(5),
                        Stroke::new(1.5, if selected { t.accent } else { t.text2 }),
                        egui::StrokeKind::Outside,
                    );
                }
                let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
                if resp.clicked() {
                    *color = Rgba((rgb << 8) | alpha as u32);
                    remember(ui, *color);
                    changed = true;
                }
            }
        });
    }
    let recent: Vec<Rgba> = ui.data(|d| d.get_temp(Id::new("recent-colors"))).unwrap_or_default();
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        for c in recent.iter().take(9) {
            let (r, resp) = ui.allocate_exact_size(vec2(20.0, 20.0), Sense::click());
            swatch(ui, t, r, *c, 4);
            if resp.on_hover_text("Recent colour").clicked() {
                *color = *c;
                changed = true;
            }
        }
        // Any colour at all.
        let mut c = c32(*color);
        if egui::widgets::color_picker::color_edit_button_srgba(ui, &mut c, egui::color_picker::Alpha::OnlyBlend)
            .on_hover_text("Custom colour")
            .changed()
        {
            *color = rgba(c);
            changed = true;
        }
    });
    ui.add_space(6.0);
    ui.label(
        egui::RichText::new("OPACITY")
            .font(theme::semibold(10.5))
            .color(t.text3),
    );
    let mut opacity = 100 - color.transparency() as i32;
    if ui.add(egui::Slider::new(&mut opacity, 0..=100).suffix("%")).changed() {
        *color = color.transp((100 - opacity) as u32);
        changed = true;
    }
    changed
}

fn remember(ui: &Ui, c: Rgba) {
    let id = Id::new("recent-colors");
    let mut recent: Vec<Rgba> = ui.data(|d| d.get_temp(id)).unwrap_or_default();
    recent.retain(|x| *x != c);
    recent.insert(0, c);
    recent.truncate(9);
    ui.data_mut(|d| d.insert_temp(id, recent));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_shape_and_mixing() {
        let g = grid();
        assert_eq!(g.len(), 8);
        assert!(g.iter().all(|r| r.len() == 10));
        assert_eq!(mix(0x000000, 0xFFFFFF, 0.5), 0x808080);
        assert_eq!(mix(0x2962FF, 0x2962FF, 0.3), 0x2962FF);
    }
}

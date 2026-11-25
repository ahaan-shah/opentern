//! The colour picker: TradingView's swatch grid (greys, then ten hues in
//! seven shades) and recent colours, or a custom colour (a saturation and
//! brightness square, a hue bar, hex and RGB fields), with an opacity
//! slider under both.

use crate::chart::style::c32;
use crate::i18n::{tr, trf};
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

/// One swatch: the colour in a rounded square with a faint edge (so dark
/// colours still read on a dark dialog). A see-through colour is shown as
/// it looks, laid over the surface (a checkerboard read as a broken image,
/// and half solid, half see-through read as two colours); the button's
/// tooltip gives the opacity.
fn swatch(ui: &Ui, t: &Theme, r: egui::Rect, color: Rgba, radius: u8) {
    let p = ui.painter();
    let c = c32(color);
    let shown = if c.a() < 255 {
        let k = c.a() as f32 / 255.0;
        motion::lerp_color(t.popup(), Color32::from_rgb(c.r(), c.g(), c.b()), k)
    } else {
        c
    };
    p.rect_filled(r, CornerRadius::same(radius), shown);
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
    let mut resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    let a = c32(*color).a();
    if a < 255 && !open {
        resp = resp.on_hover_text(trf!("{}% opacity", (a as f32 / 2.55).round()));
    }
    let mut changed = false;
    egui::Popup::from_toggle_button_response(&resp)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            changed = picker(ui, t, color);
        });
    changed
}

/// The picker's contents, for showing it outside a swatch button (stills).
pub fn show_picker(ui: &mut Ui, t: &Theme, color: &mut Rgba) -> bool {
    picker(ui, t, color)
}

fn picker(ui: &mut Ui, t: &Theme, color: &mut Rgba) -> bool {
    let mut changed = false;
    let alpha = color.a();
    // Swatches or a custom colour (remembered while the app runs).
    let mode_id = Id::new("color-picker-custom");
    let mut custom: bool = ui.data(|d| d.get_temp(mode_id)).unwrap_or(false);
    if crate::widgets::segmented(ui, t, &mut custom, &[(false, tr("Swatches")), (true, tr("Custom"))]) {
        ui.data_mut(|d| d.insert_temp(mode_id, custom));
    }
    ui.add_space(6.0);
    if custom {
        changed |= custom_picker(ui, t, color);
        changed |= opacity_slider(ui, t, color);
        return changed;
    }
    ui.spacing_mut().item_spacing = vec2(3.0, 3.0);
    for row in grid() {
        ui.horizontal(|ui| {
            for rgb in row {
                let (r, resp) = ui.allocate_exact_size(vec2(20.0, 20.0), Sense::click());
                let selected = (color.0 >> 8) == rgb;
                swatch(ui, t, r, Rgba::hex(rgb), 4);
                let hover = motion::hover(&resp);
                if selected || hover > 0.0 {
                    ui.painter().rect_stroke(
                        r.expand(1.5),
                        CornerRadius::same(5),
                        Stroke::new(
                            1.5,
                            if selected {
                                t.accent
                            } else {
                                motion::with_alpha(t.text2, hover)
                            },
                        ),
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
            if resp.clicked() {
                *color = *c;
                changed = true;
            }
        }
    });
    changed |= opacity_slider(ui, t, color);
    changed
}

fn opacity_slider(ui: &mut Ui, t: &Theme, color: &mut Rgba) -> bool {
    ui.add_space(6.0);
    ui.label(
        egui::RichText::new(tr("Opacity"))
            .font(theme::medium(12.0))
            .color(t.text2),
    );
    let mut opacity = 100 - color.transparency() as i32;
    if crate::widgets::slider(ui, t, &mut opacity, 0..=100, "%").changed() {
        *color = color.transp((100 - opacity) as u32);
        return true;
    }
    false
}

/// A small gradient image, made by `f(x, y)` (each 0–1 across the
/// image), kept until `key` changes (the square's changes with the hue).
fn gradient(
    ctx: &egui::Context,
    name: &str,
    key: u32,
    [w, h]: [usize; 2],
    f: impl Fn(f32, f32) -> Color32,
) -> egui::TextureHandle {
    let id = Id::new(("color-gradient", name));
    if let Some((k, tex)) = ctx.data(|d| d.get_temp::<(u32, egui::TextureHandle)>(id))
        && k == key
    {
        return tex;
    }
    let mut px = Vec::with_capacity(w * h);
    for j in 0..h {
        for i in 0..w {
            let x = if w > 1 { i as f32 / (w - 1) as f32 } else { 0.0 };
            let y = if h > 1 { j as f32 / (h - 1) as f32 } else { 0.0 };
            px.push(f(x, y));
        }
    }
    let img = egui::ColorImage::new([w, h], px);
    let tex = ctx.load_texture(format!("color-{name}"), img, egui::TextureOptions::LINEAR);
    ctx.data_mut(|d| d.insert_temp(id, (key, tex.clone())));
    tex
}

/// (r, g, b) of a colour.
fn rgb_of(c: Rgba) -> [u8; 3] {
    [(c.0 >> 24) as u8, (c.0 >> 16) as u8, (c.0 >> 8) as u8]
}

fn with_rgb(c: Rgba, [r, g, b]: [u8; 3]) -> Rgba {
    Rgba(((r as u32) << 24) | ((g as u32) << 16) | ((b as u32) << 8) | (c.0 & 0xFF))
}

/// "#RRGGBB" (or "RRGGBB", "#RGB") → (r, g, b).
fn parse_hex(s: &str) -> Option<[u8; 3]> {
    let h = s.trim().trim_start_matches('#');
    let h: String = if h.len() == 3 {
        h.chars().flat_map(|c| [c, c]).collect()
    } else {
        h.to_string()
    };
    if h.len() != 6 || !h.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let v = u32::from_str_radix(&h, 16).ok()?;
    Some([(v >> 16) as u8, (v >> 8) as u8, v as u8])
}

/// The custom colour: a square for saturation (across) and brightness
/// (down) at the hue from the bar under it, then hex and R, G, B fields.
/// The hue is kept apart so dragging to grey or black doesn't lose it.
fn custom_picker(ui: &mut Ui, t: &Theme, color: &mut Rgba) -> bool {
    use egui::ecolor::HsvaGamma;
    let mut changed = false;
    let rgb = rgb_of(*color);
    let solid = Color32::from_rgb(rgb[0], rgb[1], rgb[2]);
    // (hue, sat, val) as last set here, while it still gives this colour.
    let state_id = Id::new("color-picker-hsv");
    let kept: Option<([f32; 3], [u8; 3])> = ui.data(|d| d.get_temp(state_id));
    let [mut h, mut sat, mut val] = match kept {
        Some((hsv, was)) if was == rgb => hsv,
        _ => {
            let g = HsvaGamma::from(solid);
            [g.h, g.s, g.v]
        }
    };
    let w = 236.0;
    // The square.
    let (sq, resp) = ui.allocate_exact_size(vec2(w, 150.0), Sense::click_and_drag());
    {
        // A small image of the square, stretched and smoothed, so its
        // corners round like any image's (a mesh's square corners poked out).
        let key = (h * 1000.0).round() as u32;
        let tex = gradient(ui.ctx(), "sv", key, [48, 32], |x, y| {
            Color32::from(HsvaGamma {
                h,
                s: x,
                v: 1.0 - y,
                a: 1.0,
            })
        });
        ui.painter().add(
            egui::epaint::RectShape::filled(sq, CornerRadius::same(6), Color32::WHITE).with_texture(
                tex.id(),
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            ),
        );
        ui.painter().rect_stroke(
            sq,
            CornerRadius::same(6),
            Stroke::new(1.0, t.edge()),
            egui::StrokeKind::Inside,
        );
        let at = egui::pos2(sq.left() + sat * sq.width(), sq.top() + (1.0 - val) * sq.height());
        ui.painter().circle_stroke(at, 6.0, Stroke::new(2.0, Color32::WHITE));
        ui.painter()
            .circle_stroke(at, 7.0, Stroke::new(1.0, Color32::from_black_alpha(140)));
    }
    if let Some(p) = resp.interact_pointer_pos()
        && (resp.dragged() || resp.clicked() || resp.is_pointer_button_down_on())
    {
        sat = ((p.x - sq.left()) / sq.width()).clamp(0.0, 1.0);
        val = (1.0 - (p.y - sq.top()) / sq.height()).clamp(0.0, 1.0);
        changed = true;
    }
    ui.add_space(8.0);
    // The hue bar.
    let (bar, hresp) = ui.allocate_exact_size(vec2(w, 14.0), Sense::click_and_drag());
    {
        let tex = gradient(ui.ctx(), "hue", 0, [180, 1], |x, _| {
            Color32::from(HsvaGamma {
                h: x,
                s: 1.0,
                v: 1.0,
                a: 1.0,
            })
        });
        ui.painter().add(
            egui::epaint::RectShape::filled(bar, CornerRadius::same(7), Color32::WHITE).with_texture(
                tex.id(),
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            ),
        );
        let x = bar.left() + h * bar.width();
        let knob = egui::Rect::from_center_size(egui::pos2(x, bar.center().y), vec2(6.0, 18.0));
        ui.painter().rect(
            knob,
            CornerRadius::same(3),
            Color32::from(HsvaGamma {
                h,
                s: 1.0,
                v: 1.0,
                a: 1.0,
            }),
            Stroke::new(2.0, Color32::WHITE),
            egui::StrokeKind::Outside,
        );
    }
    if let Some(p) = hresp.interact_pointer_pos()
        && (hresp.dragged() || hresp.clicked() || hresp.is_pointer_button_down_on())
    {
        h = ((p.x - bar.left()) / bar.width()).clamp(0.0, 0.9999);
        changed = true;
    }
    let mut next = rgb;
    if changed {
        let c = Color32::from(HsvaGamma {
            h,
            s: sat,
            v: val,
            a: 1.0,
        });
        next = [c.r(), c.g(), c.b()];
    }
    ui.add_space(10.0);
    // Hex and R, G, B.
    let hex_id = Id::new("color-picker-hex");
    let shown_hex = format!("#{:02X}{:02X}{:02X}", next[0], next[1], next[2]);
    let mut hex: String = ui
        .data(|d| d.get_temp::<(String, [u8; 3])>(hex_id))
        .filter(|(_, was)| *was == next)
        .map(|(s, _)| s)
        .unwrap_or(shown_hex);
    let mut typed = None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let r = ui.add(
            crate::widgets::field(&mut hex)
                .desired_width(86.0)
                .font(egui::TextStyle::Monospace),
        );
        if r.changed()
            && let Some(v) = parse_hex(&hex)
        {
            typed = Some(v);
        }
        for (k, label) in ["R", "G", "B"].iter().enumerate() {
            ui.label(egui::RichText::new(*label).font(theme::medium(12.0)).color(t.text3));
            let mut v = next[k] as i32;
            if ui
                .add_sized([44.0, 32.0], egui::DragValue::new(&mut v).range(0..=255))
                .changed()
            {
                let mut n = typed.unwrap_or(next);
                n[k] = v as u8;
                typed = Some(n);
            }
        }
    });
    if let Some(v) = typed {
        next = v;
        let g = HsvaGamma::from(Color32::from_rgb(v[0], v[1], v[2]));
        // A grey keeps the hue it had.
        if g.s > 0.001 {
            h = g.h;
        }
        sat = g.s;
        val = g.v;
        changed = true;
    }
    ui.data_mut(|d| {
        d.insert_temp(state_id, ([h, sat, val], next));
        d.insert_temp(hex_id, (hex, next));
    });
    if changed && next != rgb {
        *color = with_rgb(*color, next);
        remember(ui, *color);
        return true;
    }
    false
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

    /// The custom picker draws, and a press on the square or the bar
    /// sets the colour (keeping its opacity).
    #[test]
    fn custom_picker_picks() {
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx, "Inter");
        let t = crate::theme::Theme::by_name("Tern Dark");
        let mut color = Rgba(0x2962FF80);
        let run = |events: Vec<egui::Event>, color: &mut Rgba| {
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(400.0, 400.0))),
                events,
                ..Default::default()
            };
            let mut changed = false;
            let mut out = ctx.run_ui(raw, |ui| {
                ui.data_mut(|d| d.insert_temp(Id::new("color-picker-custom"), true));
                changed = picker(ui, &t, color);
            });
            out.textures_delta.clear();
            changed
        };
        run(vec![], &mut color);
        // Near the bottom-left of the square: no saturation, almost black.
        let press = |p: egui::Pos2, down| egui::Event::PointerButton {
            pos: p,
            button: egui::PointerButton::Primary,
            pressed: down,
            modifiers: Default::default(),
        };
        let p = egui::pos2(4.0, 186.0);
        run(vec![egui::Event::PointerMoved(p)], &mut color);
        run(vec![press(p, true)], &mut color);
        run(vec![press(p, false)], &mut color);
        run(vec![], &mut color);
        assert_eq!(color.0 & 0xFF, 0x80, "opacity kept");
        let [r, g, b] = rgb_of(color);
        assert!(r.max(g).max(b) < 30, "dark after the press: {color:?}");
    }

    #[test]
    fn hex_and_rgb() {
        assert_eq!(parse_hex("#2962FF"), Some([0x29, 0x62, 0xFF]));
        assert_eq!(parse_hex("2962ff"), Some([0x29, 0x62, 0xFF]));
        assert_eq!(parse_hex("#fff"), Some([255, 255, 255]));
        assert_eq!(parse_hex("#12345"), None);
        assert_eq!(parse_hex("zzzzzz"), None);
        let c = Rgba(0x2962FF80);
        assert_eq!(rgb_of(c), [0x29, 0x62, 0xFF]);
        assert_eq!(with_rgb(c, [1, 2, 3]), Rgba(0x01020380), "keeps the opacity");
    }

    #[test]
    fn grid_shape_and_mixing() {
        let g = grid();
        assert_eq!(g.len(), 8);
        assert!(g.iter().all(|r| r.len() == 10));
        assert_eq!(mix(0x000000, 0xFFFFFF, 0.5), 0x808080);
        assert_eq!(mix(0x2962FF, 0x2962FF, 0.3), 0x2962FF);
    }
}

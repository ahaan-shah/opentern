//! The colour picker: TradingView's swatch grid (greys, then ten hues in
//! seven shades), recent colours, a custom colour and an opacity slider.

use crate::chart::style::c32;
use crate::i18n::Language;
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
    localized_color_button(ui, Language::of(ui), t, color)
}

pub(super) fn localized_color_button(ui: &mut Ui, lang: Language, t: &Theme, color: &mut Rgba) -> bool {
    let (rect, resp) = ui.allocate_exact_size(vec2(26.0, 26.0), Sense::click());
    resp.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::ColorButton,
            ui.is_enabled(),
            lang.text("Choose colour"),
        )
    });
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
            changed = picker(ui, lang, t, color);
        });
    changed
}

fn picker(ui: &mut Ui, lang: Language, t: &Theme, color: &mut Rgba) -> bool {
    let mut changed = false;
    let alpha = color.a();
    ui.spacing_mut().item_spacing = vec2(3.0, 3.0);
    for row in grid() {
        ui.horizontal(|ui| {
            for rgb in row {
                let (r, resp) = ui.allocate_exact_size(vec2(20.0, 20.0), Sense::click());
                resp.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::WidgetType::ColorButton, ui.is_enabled(), format!("#{rgb:06X}"))
                });
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
    let detail = ui.id().with("custom-colour");
    let mut expanded = ui.data(|d| d.get_temp::<bool>(detail)).unwrap_or(false);
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        for c in recent.iter().take(9) {
            let (r, resp) = ui.allocate_exact_size(vec2(20.0, 20.0), Sense::click());
            swatch(ui, t, r, *c, 4);
            let name = lang.format("Recent colour: {rgba}", &[("rgba", &format!("#{:08X}", c.0))]);
            resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::ColorButton, ui.is_enabled(), &name));
            if resp.on_hover_text(&name).clicked() {
                *color = *c;
                changed = true;
            }
        }
        // Any colour at all.
        let (rect, response) = ui.allocate_exact_size(ui.spacing().interact_size, Sense::click());
        swatch(ui, t, rect.shrink(1.0), *color, 3);
        response.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::WidgetType::ColorButton,
                ui.is_enabled(),
                lang.text("Custom colour"),
            )
        });
        if response.on_hover_text(lang.text("Custom colour")).clicked() {
            expanded = !expanded;
            ui.data_mut(|d| d.insert_temp(detail, expanded));
        }
    });
    if expanded {
        changed |= custom_editor(ui, lang, color, detail.with("hsva"));
        return changed;
    }
    ui.add_space(6.0);
    ui.label(
        egui::RichText::new(lang.text("OPACITY"))
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

/// The stock picker embeds English labels. Keep this editor in the same
/// language as the swatches, including numeric entry and clipboard help.
fn custom_editor(ui: &mut Ui, lang: Language, color: &mut Rgba, key: Id) -> bool {
    use egui::ecolor::{Hsva, HsvaGamma};
    use egui::style::NumericColorSpace;

    // Read straight RGB independently of alpha so transparent colours keep their hue.
    let from_bytes = |[r, g, b, a]: [u8; 4]| Hsva {
        a: f32::from(a) / 255.0,
        ..Hsva::from_srgb([r, g, b])
    };
    let before = *color;
    // Remember hue at zero saturation and saturation at zero brightness.
    let bytes = color.0.to_be_bytes();
    let mut hsv = ui
        .data(|d| d.get_temp::<(Rgba, HsvaGamma)>(key))
        .filter(|(saved, _)| saved == color)
        .map(|(_, hsv)| hsv)
        .unwrap_or_else(|| HsvaGamma::from(from_bytes(bytes)));
    let mut space = ui.global_style().visuals.numeric_color_space;
    ui.horizontal(|ui| {
        for (mode, label) in [
            (NumericColorSpace::GammaByte, "sRGB (0–255)"),
            (NumericColorSpace::Linear, "Linear (0–1)"),
        ] {
            if ui.selectable_value(&mut space, mode, lang.text(label)).changed() {
                ui.ctx().all_styles_mut(|s| s.visuals.numeric_color_space = space);
            }
        }
    });
    let mut exact_bytes = None;
    ui.horizontal(|ui| {
        let labels = ["Red", "Green", "Blue", "Alpha channel"];
        let mut linear = Hsva::from(hsv).to_rgba_unmultiplied();
        let mut rgba = bytes;
        let mut edited = false;
        for (i, label) in labels.iter().enumerate() {
            let drag = match space {
                NumericColorSpace::GammaByte => egui::DragValue::new(&mut rgba[i]).speed(0.5),
                NumericColorSpace::Linear => egui::DragValue::new(&mut linear[i])
                    .speed(0.003)
                    .range(0.0..=1.0)
                    .fixed_decimals(3),
            };
            let channel = ui
                .add(drag.prefix(["R ", "G ", "B ", "A "][i]))
                .on_hover_text(lang.text(label));
            ui.ctx()
                .accesskit_node_builder(channel.id, |node| node.set_label(lang.text(label)));
            edited |= channel.changed();
        }
        let copy = ui
            .button(crate::icons::ph::COPY)
            .on_hover_text(lang.text("Copy colour values"));
        ui.ctx()
            .accesskit_node_builder(copy.id, |node| node.set_label(lang.text("Copy colour values")));
        if copy.clicked() {
            let values = match space {
                NumericColorSpace::GammaByte => rgba.map(|v| v.to_string()),
                NumericColorSpace::Linear => linear.map(|v| format!("{v:.3}")),
            };
            ui.copy_text(values.join(", "));
        }
        if edited {
            let mut next = HsvaGamma::from(match space {
                NumericColorSpace::GammaByte => {
                    exact_bytes = Some(rgba);
                    from_bytes(rgba)
                }
                NumericColorSpace::Linear => Hsva::from_rgba_unmultiplied(linear[0], linear[1], linear[2], linear[3]),
            });
            if next.v == 0.0 {
                next.s = hsv.s;
            }
            if next.s == 0.0 || next.v == 0.0 {
                next.h = hsv.h;
            }
            hsv = next;
            *color = Rgba(u32::from_be_bytes(
                exact_bytes.unwrap_or_else(|| Hsva::from(hsv).to_srgba_unmultiplied()),
            ));
        }
    });
    ui.spacing_mut().slider_width = 200.0;
    egui::color_picker::show_color(ui, Color32::from(hsv), vec2(200.0, 18.0))
        .on_hover_text(lang.text("Selected colour"));
    let mut edited = saturation_value(ui, lang, &mut hsv);
    let opaque = HsvaGamma { a: 1.0, ..hsv };
    edited |= colour_slider(ui, &mut hsv.h, lang.text("Hue"), |h| {
        HsvaGamma {
            h,
            s: 1.0,
            v: 1.0,
            a: 1.0,
        }
        .into()
    });
    edited |= colour_slider(ui, &mut hsv.a, lang.text("Opacity"), |a| {
        HsvaGamma { a, ..opaque }.into()
    });
    if edited {
        *color = Rgba(u32::from_be_bytes(Hsva::from(hsv).to_srgba_unmultiplied()));
    }
    ui.data_mut(|d| d.insert_temp(key, (*color, hsv)));
    before != *color
}

/// Paint the gradient behind the native slider handle, keeping keyboard
/// adjustment, numeric entry and screen-reader controls on the same widget.
fn colour_slider(ui: &mut Ui, value: &mut f32, label: &str, at: impl Fn(f32) -> Color32) -> bool {
    ui.scope(|ui| {
        let slot = ui.painter().add(egui::Shape::Noop);
        ui.visuals_mut().widgets.inactive.bg_fill = Color32::TRANSPARENT;
        let response = ui.add(egui::Slider::new(value, 0.0..=1.0).trailing_fill(false).text(label));
        let rect = egui::Rect::from_min_size(
            egui::pos2(response.rect.left(), response.rect.center().y - 4.0),
            vec2(ui.spacing().slider_width, 8.0),
        );
        let mut mesh = egui::Mesh::default();
        const STOPS: u32 = 24;
        for i in 0..STOPS {
            let left = i as f32 / STOPS as f32;
            let right = (i + 1) as f32 / STOPS as f32;
            let x = |t| rect.left() + t * rect.width();
            for (j, v) in [(0, rect.top()), (1, rect.center().y)] {
                let bg = Color32::from_gray(if (i + j) % 2 == 0 { 192 } else { 116 });
                let base = mesh.vertices.len() as u32;
                for (t, y) in [(left, v), (right, v), (right, v + 4.0), (left, v + 4.0)] {
                    mesh.colored_vertex(egui::pos2(x(t), y), bg.blend(at(t)));
                }
                mesh.add_triangle(base, base + 1, base + 2);
                mesh.add_triangle(base, base + 2, base + 3);
            }
        }
        ui.painter().set(slot, egui::Shape::mesh(mesh));
        response.changed()
    })
    .inner
}

/// Two-dimensional saturation and brightness selection, with the hue
/// and coordinates retained even for black or transparent colours.
fn saturation_value(ui: &mut Ui, lang: Language, hsv: &mut egui::ecolor::HsvaGamma) -> bool {
    let (rect, response) = ui.allocate_exact_size(vec2(200.0, 160.0), Sense::click_and_drag());
    let label = lang.text("Saturation and brightness");
    let help = lang.text("Use Left and Right to adjust saturation, and Up and Down to adjust brightness.");
    let response = response.on_hover_text(help);
    let before = (hsv.s, hsv.v);
    if let Some(pos) = response.interact_pointer_pos() {
        response.request_focus();
        hsv.s = ((pos.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
        hsv.v = ((rect.bottom() - pos.y) / rect.height()).clamp(0.0, 1.0);
    }
    if response.has_focus() {
        ui.memory_mut(|m| {
            m.set_focus_lock_filter(
                response.id,
                egui::EventFilter {
                    horizontal_arrows: true,
                    vertical_arrows: true,
                    ..Default::default()
                },
            )
        });
        ui.input_mut(|i| {
            for (axis, minus, plus) in [
                (&mut hsv.s, egui::Key::ArrowLeft, egui::Key::ArrowRight),
                (&mut hsv.v, egui::Key::ArrowDown, egui::Key::ArrowUp),
            ] {
                let step = i32::from(i.consume_key(egui::Modifiers::NONE, plus))
                    - i32::from(i.consume_key(egui::Modifiers::NONE, minus));
                *axis = (*axis + step as f32 * 0.01).clamp(0.0, 1.0);
            }
        });
    }
    // Accessibility exposes the horizontal axis as a slider; announce
    // the second axis and how to operate it in the same control.
    ui.input(|i| {
        for req in i.accesskit_action_requests(response.id, egui::accesskit::Action::SetValue) {
            if let Some(egui::accesskit::ActionData::NumericValue(value)) = req.data {
                hsv.s = (value as f32 / 100.0).clamp(0.0, 1.0);
            }
        }
    });
    response.widget_info(|| egui::WidgetInfo::slider(ui.is_enabled(), f64::from(hsv.s) * 100.0, label));
    ui.ctx().accesskit_node_builder(response.id, |n| {
        n.set_description(help);
        n.set_value(lang.format(
            "Saturation {s}%, brightness {v}%",
            &[
                ("s", &format!("{:.0}", hsv.s * 100.0)),
                ("v", &format!("{:.0}", hsv.v * 100.0)),
            ],
        ));
        n.set_min_numeric_value(0.0);
        n.set_max_numeric_value(100.0);
        n.set_numeric_value_step(1.0);
        n.add_action(egui::accesskit::Action::SetValue);
    });
    let mut mesh = egui::Mesh::default();
    const STEPS: u32 = 12;
    for y in 0..=STEPS {
        for x in 0..=STEPS {
            let (s, v) = (x as f32 / STEPS as f32, y as f32 / STEPS as f32);
            let pos = rect.left_bottom() + vec2(s * rect.width(), -v * rect.height());
            mesh.colored_vertex(pos, egui::ecolor::HsvaGamma { s, v, a: 1.0, ..*hsv }.into());
            if x < STEPS && y < STEPS {
                let i = y * (STEPS + 1) + x;
                mesh.add_triangle(i, i + 1, i + STEPS + 1);
                mesh.add_triangle(i + 1, i + STEPS + 2, i + STEPS + 1);
            }
        }
    }
    ui.painter().add(egui::Shape::mesh(mesh));
    let pos = rect.left_bottom() + vec2(hsv.s * rect.width(), -hsv.v * rect.height());
    if response.has_focus() {
        ui.painter()
            .rect_stroke(rect, 0.0, ui.visuals().selection.stroke, egui::StrokeKind::Outside);
    }
    ui.painter().circle_stroke(pos, 5.0, Stroke::new(2.0, Color32::BLACK));
    ui.painter().circle_stroke(pos, 6.5, Stroke::new(1.0, Color32::WHITE));
    before != (hsv.s, hsv.v)
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
    fn swedish_colour_controls_edit_channels_and_keep_recent_swatches() {
        use egui::accesskit::{Action, ActionData, ActionRequest, NodeId, TreeId};

        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install_fonts(&ctx, "Inter");
        Language::Swedish.set_on(&ctx);
        let mut color = Rgba(0xFF000080);
        let frame = |color: &mut Rgba, events| {
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(800.0, 600.0))),
                events,
                ..Default::default()
            };
            let mut out = ctx.run_ui(raw, |ui| {
                color_button(ui, &theme::THEMES[0], color);
            });
            out.textures_delta.clear();
            out.platform_output.accesskit_update.unwrap().nodes
        };
        let action = |id, action, data| {
            egui::Event::AccessKitActionRequest(ActionRequest {
                target_node: id,
                target_tree: TreeId::ROOT,
                action,
                data,
            })
        };
        let node = |nodes: &[(NodeId, egui::accesskit::Node)], name: &str| {
            nodes
                .iter()
                .find(|(_, n)| n.label() == Some(name))
                .map(|(id, _)| *id)
                .unwrap_or_else(|| {
                    panic!(
                        "missing {name:?}: {:?}",
                        nodes.iter().filter_map(|(_, n)| n.label()).collect::<Vec<_>>()
                    )
                })
        };
        let mut nodes = frame(&mut color, vec![]);
        for name in ["Välj färg", "#4CAF50", "Egen färg"] {
            let click = action(node(&nodes, name), Action::Click, None);
            frame(&mut color, vec![click]);
            nodes = frame(&mut color, vec![]);
        }
        assert_eq!(color, Rgba(0x4CAF5080), "the swatch preserves the original opacity");
        node(&nodes, "Senast använd färg: #4CAF5080");
        let field = node(&nodes, "Mättnad och ljusstyrka");
        frame(&mut color, vec![action(field, Action::Focus, None)]);
        for key in [egui::Key::ArrowRight, egui::Key::ArrowDown] {
            let previous = color;
            frame(
                &mut color,
                vec![egui::Event::Key {
                    key,
                    pressed: true,
                    repeat: false,
                    physical_key: None,
                    modifiers: egui::Modifiers::NONE,
                }],
            );
            assert_ne!(previous, color, "{key:?} must adjust the colour field");
        }
        node(&nodes, "Linjärt (0–1)");
        for (label, value) in [("Röd", 123.0), ("Nyans", 0.8), ("Opacitet", 0.25)] {
            let previous = color;
            let change = action(
                node(&nodes, label),
                Action::SetValue,
                Some(ActionData::NumericValue(value)),
            );
            frame(&mut color, vec![change]);
            nodes = frame(&mut color, vec![]);
            assert_ne!(color, previous, "{label} must change the colour");
            if label == "Röd" {
                assert_eq!(color.r(), 123);
            }
        }
        assert_eq!(color.a(), 64);
        let linear = action(node(&nodes, "Linjärt (0–1)"), Action::Click, None);
        frame(&mut color, vec![linear]);
        nodes = frame(&mut color, vec![]);
        node(&nodes, "Kopiera färgvärden");
        let linear_blue = action(
            node(&nodes, "Blå"),
            Action::SetValue,
            Some(ActionData::NumericValue(1.0)),
        );
        frame(&mut color, vec![linear_blue]);
        assert_eq!(color.b(), 255);
        assert_eq!(color.a(), 64);
        assert_eq!(
            ctx.data(|d| d.get_temp::<Vec<Rgba>>(Id::new("recent-colors"))).unwrap(),
            [Rgba(0x4CAF5080)]
        );
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

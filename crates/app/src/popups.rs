//! Every menu, dialog and tooltip eases in and out. egui draws a popup whole
//! the frame it opens and drops it the frame it closes, so this works on what
//! was painted: at the end of a frame each popup layer's shapes are faded and
//! grown from where they were opened, and a closed one's last shapes are
//! painted on while they fade. Layout and input never see any of it.

use crate::motion;
use egui::emath::TSTransform;
use egui::epaint::{ClippedShape, shape_transform::adjust_colors};
use egui::layers::ShapeIdx;
use egui::{Context, Id, LayerId, Order, Pos2, Rect, Shape, vec2};
use std::collections::HashMap;
use std::sync::Arc;

const IN: f32 = 0.24;
const OUT: f32 = 0.18;

/// Areas that fade on their own, or follow something as it moves.
const NOT_OURS: [&str; 1] = ["drawing-bar"];

#[derive(Clone)]
struct Pop {
    open: bool,
    from: f32,
    start: f64,
    /// What it grows from.
    origin: Pos2,
    rect: Rect,
    shapes: Vec<ClippedShape>,
}

impl Pop {
    fn value(&self, now: f64) -> f32 {
        let (to, dur) = if self.open { (1.0, IN) } else { (0.0, OUT) };
        let p = ((now - self.start) as f32 / dur).clamp(0.0, 1.0);
        let e = if self.open {
            motion::ease_out(p)
        } else {
            motion::smooth(p)
        };
        motion::lerp(self.from, to, e)
    }

    fn turn(&mut self, open: bool, now: f64) {
        if self.open != open {
            self.from = self.value(now);
            self.open = open;
            self.start = now;
        }
    }
}

type Pops = HashMap<LayerId, Pop>;

fn store() -> Id {
    Id::new("popups")
}

/// Tests read what's painted right after a click, so there it's off unless
/// a test turns it on.
#[cfg(test)]
pub fn enable(ctx: &Context) {
    ctx.data_mut(|d| d.insert_temp(store().with("test"), true));
}

pub fn install(ctx: &Context) {
    ctx.on_end_pass("popups", Arc::new(|ui| animate(ui.ctx())));
}

fn animate(ctx: &Context) {
    #[cfg(test)]
    if ctx.data(|d| d.get_temp::<bool>(store().with("test"))).is_none() {
        return;
    }
    let now = ctx.input(|i| i.time);
    let pointer = ctx.input(|i| i.pointer.latest_pos());
    let screen = ctx.content_rect();
    let layers: Vec<LayerId> = ctx.memory(|m| {
        m.layer_ids()
            .filter(|l| matches!(l.order, Order::Foreground | Order::Tooltip))
            .filter(|l| !NOT_OURS.iter().any(|n| l.id == Id::new(*n)))
            .collect()
    });
    let mut pops: Pops = ctx.data(|d| d.get_temp(store())).unwrap_or_default();
    if pops.is_empty() && layers.is_empty() {
        return;
    }
    let mut moving = false;
    ctx.graphics_mut(|g| {
        for layer in layers {
            let list = g.get(layer).filter(|l| l.all_entries().any(|s| visible(&s.shape)));
            if let Some(list) = list {
                let shapes: Vec<ClippedShape> = list.all_entries().cloned().collect();
                let rect = bounds(&shapes, screen);
                let pop = pops.entry(layer).or_insert_with(|| Pop {
                    open: true,
                    from: 0.0,
                    start: now,
                    origin: origin(layer, pointer, rect),
                    rect,
                    shapes: Vec::new(),
                });
                pop.turn(true, now);
                pop.rect = rect;
                pop.shapes = shapes;
                let v = pop.value(now);
                if v < 1.0 {
                    moving = true;
                    let list = g.entry(layer);
                    for k in 0..pop.shapes.len() {
                        list.mutate_shape(ShapeIdx(k), |s| apply(s, layer, pop, v, screen));
                    }
                }
            } else if let Some(pop) = pops.get_mut(&layer) {
                pop.turn(false, now);
                let v = pop.value(now);
                if v <= 0.0 {
                    pops.remove(&layer);
                    continue;
                }
                moving = true;
                let list = g.entry(layer);
                for s in &pop.shapes {
                    let mut s = s.clone();
                    apply(&mut s, layer, pop, v, screen);
                    list.add(s.clip_rect, s.shape);
                }
            }
        }
    });
    if moving {
        ctx.request_repaint();
    }
    ctx.data_mut(|d| d.insert_temp(store(), pops));
}

/// A popup grows from the pointer that opened it; a dialog (or one opened
/// from the keyboard) from its middle; a tooltip stays put.
fn origin(layer: LayerId, pointer: Option<Pos2>, rect: Rect) -> Pos2 {
    match pointer {
        Some(p) if layer.id != Id::new("modal") && rect.expand(80.0).contains(p) => rect.clamp(p),
        _ => rect.center(),
    }
}

fn apply(s: &mut ClippedShape, layer: LayerId, pop: &Pop, v: f32, screen: Rect) {
    fade(&mut s.shape, v);
    // A dialog's backdrop fades where it is.
    if layer.order == Order::Tooltip || covers(&s.shape, screen) {
        return;
    }
    let scale = 0.955 + 0.045 * v;
    let origin = pop.origin.to_vec2();
    let drift = if pop.origin == pop.rect.center() {
        vec2(0.0, 8.0 * (1.0 - v))
    } else {
        vec2(0.0, 0.0)
    };
    s.transform(TSTransform::new(origin * (1.0 - scale) + drift, scale));
}

fn fade(shape: &mut Shape, a: f32) {
    match shape {
        Shape::Vec(all) => all.iter_mut().for_each(|s| fade(s, a)),
        Shape::Text(t) => t.opacity_factor *= a,
        other => adjust_colors(other, move |c| *c = c.gamma_multiply(a)),
    }
}

fn visible(s: &Shape) -> bool {
    match s {
        Shape::Noop => false,
        Shape::Vec(v) => v.iter().any(visible),
        _ => true,
    }
}

fn covers(s: &Shape, screen: Rect) -> bool {
    matches!(s, Shape::Rect(r) if r.rect.contains_rect(screen.shrink(1.0)))
}

fn bounds(shapes: &[ClippedShape], screen: Rect) -> Rect {
    let mut all = Rect::NOTHING;
    for s in shapes.iter().filter(|s| visible(&s.shape) && !covers(&s.shape, screen)) {
        let r = s.shape.visual_bounding_rect().intersect(s.clip_rect);
        if r.is_positive() {
            all = all.union(r);
        }
    }
    if all.is_positive() { all } else { screen }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(ctx: &Context, time: f64, shown: bool) -> Vec<(f32, Rect)> {
        let raw = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))),
            time: Some(time),
            ..Default::default()
        };
        let mut out = ctx.run_ui(raw, |ui| {
            if shown {
                egui::Area::new(Id::new("pop"))
                    .order(Order::Foreground)
                    .fade_in(false)
                    .fixed_pos(egui::pos2(100.0, 100.0))
                    .show(ui.ctx(), |ui| ui.label("hello"));
            }
        });
        out.textures_delta.clear();
        out.shapes
            .iter()
            .filter_map(|c| match &c.shape {
                Shape::Text(t) => Some((t.opacity_factor, c.shape.visual_bounding_rect())),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn popups_ease_in_and_out() {
        let ctx = Context::default();
        enable(&ctx);
        install(&ctx);
        let alpha = |v: &[(f32, Rect)]| v.first().map(|x| x.0);
        for k in 0..3 {
            frame(&ctx, k as f64 * 0.016, true);
        }
        let mid = alpha(&frame(&ctx, 0.08, true)).expect("shown");
        assert!(mid > 0.1 && mid < 1.0, "{mid}");
        assert_eq!(alpha(&frame(&ctx, 1.0, true)), Some(1.0));
        assert_eq!(
            alpha(&frame(&ctx, 1.1, false)),
            Some(1.0),
            "the close starts from whole"
        );
        let half = alpha(&frame(&ctx, 1.19, false)).expect("still fading");
        assert!(half > 0.1 && half < 0.9, "{half}");
        // Reopened on its way out, it goes on from there.
        let again = alpha(&frame(&ctx, 1.2, true)).expect("back");
        assert!((again - half).abs() < 0.2, "{half} → {again}");
        frame(&ctx, 2.0, true);
        frame(&ctx, 2.1, false);
        assert!(frame(&ctx, 2.5, false).is_empty(), "gone");
    }
}

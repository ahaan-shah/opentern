//! Motion primitives. Every animation in the app goes through these so
//! timings and curves stay consistent, and so repaints are only requested
//! while something is actually moving (idle = 0% CPU).

// Shared toolkit copied from Magpie; some helpers wait for the features that use them.
#![allow(dead_code)]

use egui::{Color32, Context, Id};

/// Hovers and small state changes: long enough to be seen easing.
pub const MICRO: f32 = 0.2;
pub const STANDARD: f32 = 0.22;
pub const EMPHASIS: f32 = 0.42;
pub const CHART: f32 = 0.7;

pub fn ease_out(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}

pub fn ease_out_quint(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(5)
}

pub fn ease_in_out(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    if t < 0.5 {
        4.0 * t * t * t
    } else {
        1.0 - (-2.0 * t + 2.0).powi(3) / 2.0
    }
}

/// Starts and ends softly: hovers and other small changes.
pub fn smooth(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// A slight overshoot, for things that "land" (toasts, modals).
pub fn ease_out_back(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    let c1 = 1.4;
    let c3 = c1 + 1.0;
    1.0 + c3 * (t - 1.0).powi(3) + c1 * (t - 1.0).powi(2)
}

pub fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

pub fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgba_premultiplied(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()), l(a.a(), b.a()))
}

pub fn with_alpha(c: Color32, a: f32) -> Color32 {
    c.gamma_multiply(a.clamp(0.0, 1.0))
}

#[derive(Clone, Copy)]
struct Tween {
    from: f32,
    to: f32,
    start: f64,
    dur: f32,
}

/// Smoothly animates towards `target` with an ease-out curve. Unlike egui's
/// built-in linear animation, retargeting mid-flight continues from the
/// current value, so rapidly changing numbers never jump.
pub fn tween(ctx: &Context, id: Id, target: f32, dur: f32) -> f32 {
    tween_eased(ctx, id, target, dur, ease_out)
}

fn tween_eased(ctx: &Context, id: Id, target: f32, dur: f32, ease: fn(f32) -> f32) -> f32 {
    let now = ctx.input(|i| i.time);
    let id = id.with("tween");
    let tw: Option<Tween> = ctx.data(|d| d.get_temp(id));
    let Some(mut tw) = tw else {
        ctx.data_mut(|d| {
            d.insert_temp(
                id,
                Tween {
                    from: target,
                    to: target,
                    start: now,
                    dur,
                },
            )
        });
        return target;
    };
    let p = if tw.dur <= 0.0 {
        1.0
    } else {
        ((now - tw.start) as f32 / tw.dur).clamp(0.0, 1.0)
    };
    let value = lerp(tw.from, tw.to, ease(p));
    if (target - tw.to).abs() > f32::EPSILON {
        tw = Tween {
            from: value,
            to: target,
            start: now,
            dur,
        };
        ctx.data_mut(|d| d.insert_temp(id, tw));
        ctx.request_repaint();
        return value;
    }
    if p < 1.0 {
        ctx.request_repaint();
    }
    value
}

/// Forgets a [`tween`]/[`toggle`]'s state, so its next value jumps straight
/// to the target (for changes made out of sight).
pub fn snap(ctx: &Context, id: Id) {
    ctx.data_mut(|d| d.remove::<Tween>(id.with("tween")));
}

/// Like [`tween`] but starts from `from` the first time it's seen — used for
/// things that should animate in on first appearance (chart bars growing).
pub fn tween_from(ctx: &Context, id: Id, from: f32, target: f32, dur: f32) -> f32 {
    let key = id.with("tween");
    let seen = ctx.data(|d| d.get_temp::<Tween>(key).is_some());
    if !seen {
        let now = ctx.input(|i| i.time);
        ctx.data_mut(|d| {
            d.insert_temp(
                key,
                Tween {
                    from,
                    to: target,
                    start: now,
                    dur,
                },
            )
        });
        ctx.request_repaint();
        return from;
    }
    tween(ctx, id, target, dur)
}

/// Progress 0→1 of an entrance animation that starts when a page was shown
/// (`shown_at`) plus `delay` seconds. Used for staggered card reveals.
pub fn appear(ctx: &Context, shown_at: f64, delay: f32, dur: f32) -> f32 {
    let now = ctx.input(|i| i.time);
    let p = (((now - shown_at) as f32 - delay) / dur).clamp(0.0, 1.0);
    if p < 1.0 {
        ctx.request_repaint();
    }
    ease_out(p)
}

/// Animated bool, 0.0 = false, 1.0 = true, easing at both ends.
pub fn toggle(ctx: &Context, id: Id, on: bool, dur: f32) -> f32 {
    tween_eased(ctx, id, if on { 1.0 } else { 0.0 }, dur, smooth)
}

/// How far a widget's hover look has eased in, 0–1.
pub fn hover(resp: &egui::Response) -> f32 {
    hover_if(resp, resp.hovered())
}

/// As [`hover`], for a row whose own buttons would take its hover (test the
/// pointer against its rect instead).
pub fn hover_if(resp: &egui::Response, on: bool) -> f32 {
    toggle(&resp.ctx, resp.id.with("hover-ease"), on, MICRO)
}

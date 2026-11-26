//! The favourite drawings toolbar: a floating bar over the charts with the
//! tools starred in the drawing toolbar's flyouts, one click each (as on
//! TradingView). It's off until turned on with the star in the drawing
//! toolbar. It moves by its handle (or any free spot on it), lies flat or
//! stands upright, folds down to its handle, and its tools are reordered by
//! dragging them. It shows only over the charts, and never in a snapshot.
//!
//! The bar is laid out by hand (every rect is known before anything is
//! drawn), so folding can clip and fade its body smoothly and tests can
//! find each part.

use crate::app::App;
use crate::chart::draw::{DEFAULT_FAVORITE_TOOLS, Tool};
use crate::i18n::tr;
use crate::icons::ph;
use crate::motion;
use crate::theme::{self, Theme};
use crate::widgets as w;
use egui::{Align2, CornerRadius, CursorIcon, Id, LayerId, Order, Pos2, Rect, Sense, Stroke, Ui, Vec2, vec2};
use opentern_core::Store;

/// The favourites kind the starred tools are stored under (in order).
pub const TOOL_FAVS: &str = "tool";

/// A button's side, as in the drawing toolbar (a little smaller).
const BTN: f32 = 32.0;
/// Between buttons.
const GAP: f32 = 2.0;
/// Inside the bar's edge.
const PAD: f32 = 5.0;
/// The handle's depth along the bar.
const GRIP: f32 = 22.0;
/// The divider's depth along the bar.
const RULE: f32 = 9.0;
/// Kept clear of the charts' edge.
const EDGE: f32 = 8.0;
const RADIUS: u8 = 12;

/// What the bar keeps between frames. Where it sits and how it looks are
/// in the arrangement (`Arrangement::fav_toolbar*`), saved with the rest of
/// the layout; the favourites themselves are in the workspace.
#[derive(Default)]
pub struct FavBar {
    /// The frame (pass) the charts were last drawn in: the bar shows only
    /// over them, so not over Settings, the overview or a maximized panel.
    pub charts_pass: u64,
    /// The tool being dragged to a new place (its index), while it is.
    drag: Option<usize>,
    /// Where things were laid out last frame (`Rect::NOTHING` when hidden).
    pub at: Parts,
}

/// The bar's parts as laid out last frame.
#[derive(Clone, Debug, PartialEq)]
pub struct Parts {
    pub bar: Rect,
    pub grip: Rect,
    pub tools: Vec<Rect>,
    pub orient: Rect,
    pub collapse: Rect,
    pub close: Rect,
}

impl Default for Parts {
    fn default() -> Parts {
        Parts {
            bar: Rect::NOTHING,
            grip: Rect::NOTHING,
            tools: Vec::new(),
            orient: Rect::NOTHING,
            collapse: Rect::NOTHING,
            close: Rect::NOTHING,
        }
    }
}

/// A new workspace starts with a few favourites (once: emptying the list
/// later sticks).
pub fn seed_favorites(store: &mut Store) {
    const SEEDED: &str = "fav_tools_seeded";
    if store.value(SEEDED).is_some() {
        return;
    }
    if store.favorites(TOOL_FAVS).is_empty() {
        let keys: Vec<String> = DEFAULT_FAVORITE_TOOLS.iter().map(|t| t.key()).collect();
        if let Err(e) = store.set_favorites(TOOL_FAVS, &keys) {
            crate::diag::crumb(format!("couldn't save favourite tools: {e}"));
        }
    }
    let _ = store.set_value(SEEDED, "1");
}

/// Lengths along the bar's long side.
struct Lengths {
    /// The body (tools, divider, orientation), unfolded.
    body: f32,
    /// The whole bar unfolded.
    full: f32,
    /// The whole bar as it is (`fold` 0 = open, 1 = folded).
    now: f32,
}

fn lengths(body: f32, fold: f32) -> Lengths {
    // Handle, body, then fold and close.
    let fixed = PAD + GRIP + GAP + BTN + GAP + BTN + PAD;
    Lengths {
        body,
        full: fixed + body,
        now: fixed + body * (1.0 - fold),
    }
}

impl App {
    /// The starred tools, in the bar's order.
    pub fn favorite_tools(&self) -> Vec<Tool> {
        let mut tools = Vec::new();
        for key in self.store.favorites(TOOL_FAVS) {
            if let Some(tool) = Tool::from_key(key)
                && !tools.contains(&tool)
            {
                tools.push(tool);
            }
        }
        tools
    }

    pub fn is_tool_favorite(&self, tool: Tool) -> bool {
        self.store.is_favorite(TOOL_FAVS, &tool.key())
    }

    /// Stars or unstars a tool. Starring one shows the bar, so it's clear
    /// where it went.
    pub fn toggle_favorite_tool(&mut self, tool: Tool) {
        if !self.is_tool_favorite(tool) {
            self.layout.fav_toolbar = true;
        }
        self.toggle_favorite(TOOL_FAVS, &tool.key());
    }

    pub fn set_favorite_tools(&mut self, tools: &[Tool]) {
        let mut keys: Vec<String> = Vec::new();
        for t in tools {
            let k = t.key();
            if !keys.contains(&k) {
                keys.push(k);
            }
        }
        if let Err(e) = self.store.set_favorites(TOOL_FAVS, &keys) {
            crate::diag::crumb(format!("couldn't save favourite tools: {e}"));
        }
    }

    /// Takes the tool at `from` out and puts it back at `to` (an index in
    /// the list without it).
    pub fn move_favorite_tool(&mut self, from: usize, to: usize) {
        let mut tools = self.favorite_tools();
        if from >= tools.len() || to >= tools.len() || from == to {
            return;
        }
        let tool = tools.remove(from);
        tools.insert(to, tool);
        self.set_favorite_tools(&tools);
    }

    /// Whether the charts are on screen this frame, for what floats over
    /// them (this bar, the selected drawing's): not when Settings, the
    /// overview or a maximized panel has their place, under a dialog, or
    /// while a snapshot is being taken (so it shows the chart alone).
    pub(crate) fn charts_on_screen(&self, ctx: &egui::Context) -> bool {
        self.favbar.charts_pass == ctx.cumulative_pass_nr()
            && self.modal.is_none()
            && self.snap.is_none()
            && self.grid_rect.is_positive()
    }

    /// The floating bar, drawn after the charts.
    pub(crate) fn favorite_toolbar(&mut self, ctx: &egui::Context) {
        // Only where the charts are, and gone at once (no fade) when they
        // aren't, or while a snapshot is being taken.
        if !self.charts_on_screen(ctx) {
            self.favbar.at = Parts::default();
            self.favbar.drag = None;
            return;
        }
        let shown = self.layout.fav_toolbar;
        let k = motion::toggle(ctx, Id::new("fav-toolbar-shown"), shown, motion::STANDARD);
        if k <= 0.0 {
            self.favbar.at = Parts::default();
            self.favbar.drag = None;
            return;
        }
        let t = self.t();
        let tools = self.favorite_tools();
        let fold = motion::toggle(
            ctx,
            Id::new("fav-toolbar-fold"),
            self.layout.fav_toolbar_collapsed,
            motion::STANDARD,
        );
        let step = BTN + GAP;
        let label_w = if tools.is_empty() {
            let g =
                ctx.fonts_mut(|f| f.layout_no_wrap(tr("No favorites yet").to_string(), theme::regular(12.5), t.text3));
            g.size().x + 16.0
        } else {
            0.0
        };
        // Upright, an empty bar shows a star instead of the words.
        let empty_w = |vertical: bool| if vertical { BTN } else { label_w };
        let tools_len = |vertical: bool| {
            if tools.is_empty() {
                empty_w(vertical) + GAP
            } else {
                tools.len() as f32 * step
            }
        };
        let room = self.grid_rect.shrink(EDGE);
        // Lying flat or upright as chosen, unless only the other way fits
        // (a narrow chart, a short window); the choice itself stays.
        let full = |vertical: bool| lengths(tools_len(vertical) + RULE + BTN + GAP, 0.0).full;
        let wanted = self.layout.fav_toolbar_vertical;
        let fits = |vertical: bool| {
            if vertical {
                full(true) <= room.height()
            } else {
                full(false) <= room.width()
            }
        };
        let vertical = if fits(wanted) || !fits(!wanted) {
            wanted
        } else {
            !wanted
        };
        let empty_w = empty_w(vertical);
        let tools_len = tools_len(vertical);
        let len = lengths(tools_len + RULE + BTN + GAP, fold);
        let thick = BTN + 2.0 * PAD;
        let size = |l: f32| if vertical { vec2(thick, l) } else { vec2(l, thick) };

        // Where it sits: a fraction of the room it can move in, measured
        // with its unfolded size, so folding keeps the handle in place.
        let travel = (room.size() - size(len.full)).max(Vec2::ZERO);
        let at = self.layout.fav_toolbar_at.unwrap_or_else(|| {
            if vertical {
                [0.0, 0.5]
            } else {
                // Centred, clear of the time scale.
                let above = if travel.y > 0.0 { 1.0 - 40.0 / travel.y } else { 1.0 };
                [0.5, above.clamp(0.0, 1.0)]
            }
        });
        let origin = room.min + vec2(at[0].clamp(0.0, 1.0) * travel.x, at[1].clamp(0.0, 1.0) * travel.y);
        let bar = Rect::from_min_size(origin, size(len.now));
        // A slot `from` along the long side, `l` long, across the buttons.
        let slot = |from: f32, l: f32| {
            if vertical {
                Rect::from_min_size(origin + vec2(PAD, from), vec2(BTN, l))
            } else {
                Rect::from_min_size(origin + vec2(from, PAD), vec2(l, BTN))
            }
        };
        let grip = slot(PAD, GRIP);
        let body_at = PAD + GRIP + GAP;
        let body_full = slot(body_at, len.body);
        let body_now = slot(body_at, len.body * (1.0 - fold));
        let tool_rects: Vec<Rect> = (0..tools.len()).map(|i| slot(body_at + i as f32 * step, BTN)).collect();
        let rule = slot(body_at + tools_len, RULE);
        let orient = slot(body_at + tools_len + RULE, BTN);
        let tail = body_at + len.body * (1.0 - fold);
        let collapse = slot(tail, BTN);
        let close = slot(tail + step, BTN);

        let mut moved: Option<Vec2> = None;
        let mut pick: Option<Tool> = None;
        let mut unstar: Option<Tool> = None;
        let mut reorder: Option<(usize, usize)> = None;
        let mut flip = false;
        let mut fold_now = false;
        let mut hide = false;
        let dragging = self.favbar.drag;
        let mut drag_start: Option<usize> = None;
        let mut drag_end = false;

        egui::Area::new(Id::new("fav-toolbar"))
            .order(Order::Foreground)
            .fixed_pos(origin)
            .constrain(false)
            .interactable(shown)
            .show(ctx, |ui| {
                ui.multiply_opacity(k);
                // Nothing spills over the panels beside the charts, even
                // when neither way fits.
                ui.set_clip_rect(self.grid_rect);
                ui.allocate_rect(bar, Sense::hover());
                let p = ui.painter();
                let radius = CornerRadius::same(RADIUS);
                p.add(ui.visuals().popup_shadow.as_shape(bar, radius));
                p.rect(
                    bar,
                    radius,
                    t.popup(),
                    Stroke::new(1.0, t.edge()),
                    egui::StrokeKind::Inside,
                );

                // The bar moves when dragged anywhere a button isn't; the
                // handle shows where.
                let back = ui.interact(bar, Id::new("fav-toolbar-move"), Sense::drag());
                let lit = motion::toggle(
                    ui.ctx(),
                    Id::new("fav-toolbar-grip"),
                    back.hovered() || back.dragged(),
                    motion::MICRO,
                );
                ui.painter().text(
                    grip.center(),
                    Align2::CENTER_CENTER,
                    if vertical { ph::DOTS_SIX } else { ph::DOTS_SIX_VERTICAL },
                    theme::regular(18.0),
                    motion::lerp_color(t.text3, t.text, lit),
                );
                if back.dragged() {
                    moved = Some(back.drag_delta());
                }
                back.on_hover_cursor(if moved.is_some() {
                    CursorIcon::Grabbing
                } else {
                    CursorIcon::Grab
                });

                // The body (tools, divider, orientation) folds away into the
                // handle: clipped to what's left of it, and fading.
                let mut body = ui.new_child(egui::UiBuilder::new().max_rect(body_full));
                body.set_clip_rect(body_now.intersect(ui.clip_rect()));
                body.multiply_opacity(1.0 - fold);
                if fold < 1.0 {
                    if tools.is_empty() {
                        let r = slot(body_at, empty_w);
                        if vertical {
                            let resp = body.interact(r, Id::new("fav-toolbar-empty"), Sense::hover());
                            body.painter().text(
                                r.center(),
                                Align2::CENTER_CENTER,
                                ph::STAR,
                                theme::regular(16.0),
                                t.text3,
                            );
                            resp.on_hover_text(tr("No favorites yet"));
                        } else {
                            body.painter().text(
                                r.center(),
                                Align2::CENTER_CENTER,
                                tr("No favorites yet"),
                                theme::regular(12.5),
                                t.text3,
                            );
                        }
                    }
                    for (i, (&tool, &r)) in tools.iter().zip(&tool_rects).enumerate() {
                        let on = self.layout.tool == tool;
                        let tip = self.keys.tip(tr(tool.name()), crate::keys::Action::Tool(tool));
                        let lifted = dragging == Some(i);
                        let mut cell = body.new_child(egui::UiBuilder::new().max_rect(r));
                        if lifted {
                            cell.multiply_opacity(0.3);
                        }
                        let id = Id::new(("fav-toolbar-tool", tool.key()));
                        let resp = {
                            let resp = cell.interact(r, id, Sense::click_and_drag());
                            w::tool_button_look(&cell, &t, resp, tool.icon(), &tip, on)
                        };
                        if resp.drag_started() {
                            drag_start = Some(i);
                        }
                        if lifted && resp.drag_stopped() {
                            drag_end = true;
                        }
                        if resp.clicked() {
                            pick = Some(if on { Tool::Cross } else { tool });
                        }
                        egui::Popup::context_menu(&resp).show(|ui| {
                            ui.set_width(200.0);
                            ui.spacing_mut().item_spacing.y = 1.0;
                            if w::menu_item(ui, &t, Some(ph::STAR), tr("Remove from favorites"), "", false).clicked() {
                                unstar = Some(tool);
                                ui.close();
                            }
                            let (back_label, back_icon, on_label, on_icon) = if vertical {
                                (tr("Move up"), ph::ARROW_UP, tr("Move down"), ph::ARROW_DOWN)
                            } else {
                                (tr("Move left"), ph::ARROW_LEFT, tr("Move right"), ph::ARROW_RIGHT)
                            };
                            if i > 0 && w::menu_item(ui, &t, Some(back_icon), back_label, "", false).clicked() {
                                reorder = Some((i, i - 1));
                                ui.close();
                            }
                            if i + 1 < tools.len() && w::menu_item(ui, &t, Some(on_icon), on_label, "", false).clicked()
                            {
                                reorder = Some((i, i + 1));
                                ui.close();
                            }
                        });
                    }
                    rule_line(&body, &t, rule, vertical);
                    let (glyph, tip) = if vertical {
                        (ph::ROWS, tr("Horizontal toolbar"))
                    } else {
                        (ph::COLUMNS, tr("Vertical toolbar"))
                    };
                    if button(&mut body, &t, orient, "fav-toolbar-orient", glyph, tip).clicked() {
                        flip = true;
                    }
                }

                // Fold and close stay, after the body.
                let (glyph, tip) = match (vertical, fold > 0.5) {
                    (false, false) => (ph::CARET_LEFT, tr("Collapse toolbar")),
                    (false, true) => (ph::CARET_RIGHT, tr("Expand toolbar")),
                    (true, false) => (ph::CARET_UP, tr("Collapse toolbar")),
                    (true, true) => (ph::CARET_DOWN, tr("Expand toolbar")),
                };
                if button(ui, &t, collapse, "fav-toolbar-fold-btn", glyph, tip).clicked() {
                    fold_now = true;
                }
                if button(ui, &t, close, "fav-toolbar-close", ph::X, tr("Hide toolbar")).clicked() {
                    hide = true;
                }

                // A tool on the move: it follows the pointer, and a line
                // marks where it would land.
                if let Some(from) = dragging
                    && from < tool_rects.len()
                    && let Some(pointer) = ui.ctx().pointer_interact_pos()
                {
                    let to = drop_slot(&tool_rects, from, pointer, vertical);
                    if to != from {
                        let others: Vec<Rect> = tool_rects
                            .iter()
                            .enumerate()
                            .filter(|(j, _)| *j != from)
                            .map(|(_, r)| *r)
                            .collect();
                        let line = if to == 0 {
                            if vertical {
                                others[0].top() - GAP / 2.0
                            } else {
                                others[0].left() - GAP / 2.0
                            }
                        } else if vertical {
                            others[to - 1].bottom() + GAP / 2.0
                        } else {
                            others[to - 1].right() + GAP / 2.0
                        };
                        let stroke = Stroke::new(2.0, t.accent);
                        let r = tool_rects[0];
                        if vertical {
                            ui.painter().hline(r.x_range().shrink(3.0), line, stroke);
                        } else {
                            ui.painter().vline(line, r.y_range().shrink(3.0), stroke);
                        }
                    }
                    let ghost = Rect::from_center_size(pointer, vec2(BTN, BTN));
                    let lp = ui
                        .ctx()
                        .layer_painter(LayerId::new(Order::Tooltip, Id::new("fav-toolbar-ghost")));
                    lp.add(ui.visuals().popup_shadow.as_shape(ghost, CornerRadius::same(8)));
                    lp.rect_filled(ghost, CornerRadius::same(8), t.accent_soft());
                    lp.text(
                        ghost.center(),
                        Align2::CENTER_CENTER,
                        tools[from].icon(),
                        theme::regular(BTN * 0.56),
                        t.accent,
                    );
                    ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
                }
            });

        self.favbar.at = Parts {
            bar,
            grip,
            tools: if fold < 1.0 { tool_rects.clone() } else { Vec::new() },
            orient,
            collapse,
            close,
        };

        // A tool dropped somewhere new.
        if let Some(from) = self.favbar.drag {
            let released = ctx.input(|i| !i.pointer.any_down());
            if drag_end || released {
                if let Some(pointer) = ctx.pointer_interact_pos().or(ctx.input(|i| i.pointer.latest_pos()))
                    && from < tool_rects.len()
                {
                    let to = drop_slot(&tool_rects, from, pointer, vertical);
                    reorder = reorder.or(Some((from, to)));
                }
                self.favbar.drag = None;
            }
        }
        if let Some(i) = drag_start {
            self.favbar.drag = Some(i);
        }
        if let Some(d) = moved {
            // Back to a fraction of the room, kept in memory: the layout is
            // saved with the window, not each frame.
            let p = origin + d - room.min;
            let f = |v: f32, room: f32| if room > 0.0 { (v / room).clamp(0.0, 1.0) } else { 0.0 };
            self.layout.fav_toolbar_at = Some([f(p.x, travel.x), f(p.y, travel.y)]);
        }
        if let Some(tool) = pick {
            self.pick_tool(tool);
        }
        if let Some(tool) = unstar {
            self.toggle_favorite_tool(tool);
        }
        if let Some((from, to)) = reorder {
            self.move_favorite_tool(from, to);
        }
        if flip {
            // From the way it shows (which may not be the one chosen).
            self.layout.fav_toolbar_vertical = !vertical;
        }
        if fold_now {
            self.layout.fav_toolbar_collapsed = !self.layout.fav_toolbar_collapsed;
        }
        if hide {
            self.layout.fav_toolbar = false;
        }
    }
}

/// Where the tool at `from` would land if let go at `pointer`: its index in
/// the list without it (the number of other tools before the pointer).
fn drop_slot(rects: &[Rect], from: usize, pointer: Pos2, vertical: bool) -> usize {
    rects
        .iter()
        .enumerate()
        .filter(|(j, _)| *j != from)
        .filter(|(_, r)| {
            if vertical {
                r.center().y < pointer.y
            } else {
                r.center().x < pointer.x
            }
        })
        .count()
}

/// One of the bar's own buttons (orientation, fold, close).
fn button(ui: &mut Ui, t: &Theme, rect: Rect, id: &str, glyph: &str, tip: &str) -> egui::Response {
    let resp = ui.interact(rect, Id::new(id), Sense::click());
    w::tool_button_look(ui, t, resp, glyph, tip, false)
}

fn rule_line(ui: &Ui, t: &Theme, r: Rect, vertical: bool) {
    let stroke = Stroke::new(1.0, t.border);
    if vertical {
        ui.painter().hline(r.x_range().shrink(6.0), r.center().y, stroke);
    } else {
        ui.painter().vline(r.center().x, r.y_range().shrink(6.0), stroke);
    }
}

#[cfg(test)]
mod tests;

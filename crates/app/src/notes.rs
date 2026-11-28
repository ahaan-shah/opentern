//! Notifications: what happened while you were looking elsewhere (alerts
//! fired, a snapshot saved, an update available, a long run finished), in
//! the Notifications widget; each pops up as a card for a few seconds.

use crate::app::App;
use crate::i18n::{tr, trm};
use crate::icons::ph;
use crate::theme::{self, Theme};
use egui::{CornerRadius, Sense, Ui, pos2, vec2};
use std::time::Instant;

#[derive(Clone, Debug)]
pub struct Note {
    pub time: i64,
    pub icon: &'static str,
    pub title: String,
    pub body: String,
    /// A file or folder the note is about (opened on click).
    pub path: Option<std::path::PathBuf>,
}

#[derive(Default)]
pub struct Notes {
    pub list: Vec<Note>,
    pub unread: usize,
    toasts: Vec<(Note, Instant)>,
}

impl App {
    /// Adds a notification, and pops it up for a few seconds.
    pub fn notify(&mut self, icon: &'static str, title: &str, body: &str, path: Option<std::path::PathBuf>) {
        let n = Note {
            time: jiff::Timestamp::now().as_millisecond(),
            icon,
            title: title.into(),
            body: body.into(),
            path,
        };
        self.notes.toasts.push((n.clone(), Instant::now()));
        self.notes.list.insert(0, n);
        self.notes.list.truncate(200);
        if self.layout.widget != Some(crate::shell::Widget::Notifications) {
            self.notes.unread += 1;
        }
        self.egui_ctx.request_repaint();
    }

    /// Every notification pops up as a small card at the top right, in
    /// the dialogs' look (their border, glow and shadow), newest on top. A
    /// card stays while the pointer is on it; a click opens what it's
    /// about, or puts it away.
    pub(crate) fn note_toasts(&mut self, ctx: &egui::Context) {
        const SHOWN: f32 = 6.0;
        self.notes.toasts.retain(|(_, t)| t.elapsed().as_secs_f32() < SHOWN);
        if self.notes.toasts.is_empty() {
            return;
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(50));
        let t = self.t();
        const W: f32 = 320.0;
        // The top right of the charts: clear of the side panel and its
        // rail, under the top bar (the screen's corner before a layout).
        let area = if self.grid_rect.is_positive() {
            self.grid_rect
        } else {
            ctx.content_rect().shrink2(vec2(64.0, 44.0))
        };
        // Left of the price scale, so the last price stays in sight.
        let right = area.right() - 92.0;
        let mut y = area.top() + 12.0;
        let mut open = None;
        let mut close = None;
        let mut held = None;
        let n = self.notes.toasts.len();
        for (k, (note, born)) in self.notes.toasts.iter().enumerate().rev().take(4) {
            let age = born.elapsed().as_secs_f32();
            let alpha = (age / 0.15).min((SHOWN - age) / 0.5).clamp(0.0, 1.0);
            let area = egui::Area::new(egui::Id::new(("note-card", k, note.time)))
                .fixed_pos(pos2(right - W, y))
                .order(egui::Order::Foreground)
                .show(ctx, |ui| {
                    ui.set_opacity(alpha);
                    card(ui, &t, note, k + 1 == n)
                });
            let hovered = area.response.contains_pointer();
            if hovered {
                held = Some(k);
            }
            match area.inner {
                Some(true) => close = Some(k),
                Some(false) if area.response.interact(Sense::click()).clicked() => open = Some(k),
                _ => {}
            }
            y += area.response.rect.height() + 10.0;
        }
        if let Some(k) = held {
            // Kept fully shown while it's being read.
            self.notes.toasts[k].1 = Instant::now() - std::time::Duration::from_millis(200);
        }
        if let Some(k) = close.or(open) {
            let (note, _) = self.notes.toasts.remove(k);
            if open.is_some()
                && let Some(p) = note.path
            {
                crate::app::open_path(&p);
            }
        }
    }

    pub(crate) fn notifications_widget(&mut self, ui: &mut Ui, t: &Theme) {
        self.notes.unread = 0;
        if self.notes.list.is_empty() {
            crate::widgets::empty_state(ui, t, crate::icons::ph::BELL_SIMPLE, tr("All caught up"));
            return;
        }
        let mut open = None;
        egui::ScrollArea::vertical()
            .id_salt("notes")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                // Rounded rows, as the watchlist's; no rules between them.
                ui.spacing_mut().item_spacing.y = 2.0;
                let w = ui.available_width();
                for n in &self.notes.list {
                    let when = jiff::Timestamp::from_millisecond(n.time)
                        .map(|ts| ts.to_zoned(jiff::tz::TimeZone::system()).strftime("%H:%M").to_string())
                        .unwrap_or_default();
                    // Icon in a soft disc; title and time on one line, the
                    // body under the title, aligned with it.
                    let text_x = 44.0;
                    let body = (!n.body.is_empty()).then(|| {
                        ui.painter().layout(
                            n.body.clone(),
                            theme::regular(12.0),
                            t.text2,
                            (w - text_x - 10.0).max(60.0),
                        )
                    });
                    let h = 16.0 + 18.0 + body.as_ref().map(|g| g.size().y + 2.0).unwrap_or(0.0) + 12.0;
                    let (r, resp) = ui.allocate_exact_size(vec2(w, h), Sense::click());
                    let p = ui.painter();
                    let hover = crate::motion::hover_if(&resp, ui.rect_contains_pointer(r) && n.path.is_some());
                    if hover > 0.0 {
                        p.rect_filled(r, 8, crate::motion::with_alpha(t.hover_wash(), hover));
                    }
                    let disc = egui::pos2(r.left() + 20.0, r.top() + 14.0 + 11.0);
                    p.circle_filled(disc, 14.0, t.accent_soft());
                    p.text(
                        disc,
                        egui::Align2::CENTER_CENTER,
                        n.icon,
                        theme::regular(14.0),
                        t.accent,
                    );
                    let ty = r.top() + 14.0 + 9.0;
                    let time = p.text(
                        egui::pos2(r.right() - 10.0, ty),
                        egui::Align2::RIGHT_CENTER,
                        when,
                        theme::regular(11.5),
                        t.text3,
                    );
                    let font = theme::semibold(12.5);
                    let title = crate::format::elide_to(p, &n.title, &font, time.left() - (r.left() + text_x) - 8.0);
                    p.text(
                        egui::pos2(r.left() + text_x, ty),
                        egui::Align2::LEFT_CENTER,
                        title,
                        font,
                        t.text,
                    );
                    if let Some(g) = body {
                        p.galley(egui::pos2(r.left() + text_x, ty + 11.0), g, t.text2);
                    }
                    if resp.clicked()
                        && let Some(p) = &n.path
                    {
                        open = Some(p.clone());
                    }
                }
            });
        if let Some(p) = open {
            crate::app::open_path(&p);
        }
        let _ = ph::BELL;
    }
}

/// One notification card: its icon in a soft disc, the title, the text
/// under it, and × on hover. Returns `Some(true)` when × was pressed.
fn card(ui: &mut Ui, t: &Theme, n: &Note, _newest: bool) -> Option<bool> {
    const W: f32 = 320.0;
    let mut closed = false;
    let frame = egui::Frame::new()
        .fill(t.popup())
        .stroke(crate::dialogs::border_stroke(t))
        .corner_radius(CornerRadius::same(14))
        .inner_margin(egui::Margin::same(14))
        .shadow(egui::Shadow {
            offset: [0, 12],
            blur: 32,
            spread: 0,
            color: t.shadow(),
        });
    let resp = frame.show(ui, |ui| {
        let slots = crate::dialogs::sheen_slots(ui);
        ui.set_width(W - 28.0);
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 12.0;
            let (disc, _) = ui.allocate_exact_size(vec2(30.0, 30.0), Sense::hover());
            ui.painter().circle_filled(disc.center(), 15.0, t.accent_soft());
            ui.painter().text(
                disc.center(),
                egui::Align2::CENTER_CENTER,
                n.icon,
                theme::regular(15.0),
                t.accent,
            );
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 3.0;
                ui.set_width(W - 28.0 - 42.0 - 22.0);
                ui.add_space(if n.body.is_empty() { 7.0 } else { 1.0 });
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(trm(&n.title))
                            .font(theme::semibold(13.5))
                            .color(t.text),
                    )
                    .wrap(),
                );
                if !n.body.is_empty() {
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(trm(&n.body))
                                .font(theme::regular(12.5))
                                .color(t.text2),
                        )
                        .wrap(),
                    );
                }
            });
        });
        let inside = ui.min_rect().expand(14.0);
        crate::dialogs::sheen(ui, t, &slots[..slots.len() - 1], inside, 13);
        let border = crate::dialogs::border_stroke(t);
        ui.painter().set(
            slots[slots.len() - 1],
            egui::Shape::Rect(egui::epaint::RectShape::new(
                inside.expand(border.width),
                CornerRadius::same(14),
                egui::Color32::TRANSPARENT,
                border,
                egui::StrokeKind::Inside,
            )),
        );
        // × in the corner while the pointer is on the card.
        if ui.rect_contains_pointer(inside) {
            let x = egui::Rect::from_center_size(inside.right_top() + vec2(-18.0, 18.0), vec2(22.0, 22.0));
            let r = ui.interact(x, ui.id().with(("note-x", n.time)), Sense::click());
            let c = crate::motion::lerp_color(t.text3, t.text, crate::motion::hover(&r));
            ui.painter()
                .text(x.center(), egui::Align2::CENTER_CENTER, ph::X, theme::regular(13.0), c);
            closed = r.clicked();
        }
    });
    let _ = resp;
    Some(closed)
}

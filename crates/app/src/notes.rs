//! Notifications: what happened while you were looking elsewhere (alerts
//! fired, a snapshot saved, an update available, a long run finished), in
//! the Notifications widget, with a short toast for the ones you asked for.

use crate::app::App;
use crate::icons::ph;
use crate::theme::{self, Theme};
use egui::{CornerRadius, Sense, Stroke, Ui, pos2, vec2};
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
    /// Adds a notification; `toast` also shows it briefly over the chart.
    pub fn notify(
        &mut self,
        icon: &'static str,
        title: &str,
        body: &str,
        path: Option<std::path::PathBuf>,
        toast: bool,
    ) {
        let n = Note {
            time: jiff::Timestamp::now().as_millisecond(),
            icon,
            title: title.into(),
            body: body.into(),
            path,
        };
        if toast {
            self.notes.toasts.push((n.clone(), Instant::now()));
        }
        self.notes.list.insert(0, n);
        self.notes.list.truncate(200);
        if self.layout.widget != Some(crate::shell::Widget::Notifications) {
            self.notes.unread += 1;
        }
        self.egui_ctx.request_repaint();
    }

    /// Toasts, bottom right over the chart, for a few seconds.
    pub(crate) fn note_toasts(&mut self, ctx: &egui::Context) {
        self.notes.toasts.retain(|(_, t)| t.elapsed().as_secs_f32() < 5.0);
        if self.notes.toasts.is_empty() {
            return;
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(100));
        let t = self.t();
        let screen = ctx.content_rect();
        let mut y = screen.bottom() - 120.0;
        let mut open = None;
        for (k, (n, born)) in self.notes.toasts.iter().enumerate().rev().take(3) {
            let age = born.elapsed().as_secs_f32();
            let alpha = ((5.0 - age) / 0.5).clamp(0.0, 1.0);
            let w = 300.0;
            let area = egui::Area::new(egui::Id::new(("note-toast", k, n.time)))
                .pivot(egui::Align2::LEFT_BOTTOM)
                .fixed_pos(pos2(screen.right() - w - 60.0, y))
                .order(egui::Order::Foreground)
                .show(ctx, |ui| {
                    ui.set_opacity(alpha);
                    egui::Frame::new()
                        .fill(t.elevated)
                        .stroke(Stroke::new(1.0, t.border))
                        .corner_radius(CornerRadius::same(10))
                        .inner_margin(egui::Margin::symmetric(14, 10))
                        .show(ui, |ui| {
                            ui.set_width(w - 28.0);
                            ui.horizontal(|ui| {
                                ui.label(egui::RichText::new(n.icon).font(theme::regular(15.0)).color(t.accent));
                                ui.label(egui::RichText::new(&n.title).font(theme::semibold(13.0)));
                            });
                            if !n.body.is_empty() {
                                ui.label(egui::RichText::new(&n.body).color(t.text2).size(12.5));
                            }
                        })
                });
            if area.response.interact(Sense::click()).clicked() {
                open = Some(k);
            }
            y -= area.response.rect.height() + 8.0;
        }
        if let Some(k) = open {
            let (n, _) = self.notes.toasts.remove(k);
            if let Some(p) = n.path {
                crate::app::open_path(&p);
            }
        }
    }

    pub(crate) fn notifications_widget(&mut self, ui: &mut Ui, t: &Theme) {
        self.notes.unread = 0;
        if self.notes.list.is_empty() {
            crate::widgets::empty_state(ui, t, crate::icons::ph::BELL_SIMPLE, "All caught up");
            return;
        }
        let mut open = None;
        egui::ScrollArea::vertical()
            .id_salt("notes")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
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
                    if ui.rect_contains_pointer(r) && n.path.is_some() {
                        p.rect_filled(r, 8, t.hover_wash());
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
                    p.hline(
                        (r.left() + text_x)..=r.right(),
                        r.bottom() - 0.5,
                        egui::Stroke::new(1.0, t.border.gamma_multiply(0.6)),
                    );
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

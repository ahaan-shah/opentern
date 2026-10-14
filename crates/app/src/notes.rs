//! Notifications: what happened while you were looking elsewhere (alerts
//! fired, a snapshot saved, an update available, a long run finished), in
//! the Notifications widget, with a short toast for the ones you asked for.

use crate::app::App;
use crate::icons::ph;
use crate::theme::{self, Theme};
use egui::{CornerRadius, Sense, Stroke, Ui, pos2};
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
            crate::widgets::empty_state(
                ui,
                t,
                crate::icons::ph::BELL_SIMPLE,
                "All caught up",
                "Alerts that fire, snapshots and finished runs show up here.",
            );
            return;
        }
        ui.horizontal(|ui| {
            if ui.small_button("Clear").clicked() {
                self.notes.list.clear();
            }
        });
        ui.add_space(4.0);
        let mut open = None;
        egui::ScrollArea::vertical()
            .id_salt("notes")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for n in &self.notes.list {
                    let when = jiff::Timestamp::from_millisecond(n.time)
                        .map(|ts| ts.to_zoned(jiff::tz::TimeZone::system()).strftime("%H:%M").to_string())
                        .unwrap_or_default();
                    let r = egui::Frame::new()
                        .inner_margin(egui::Margin::symmetric(4, 6))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.horizontal(|ui| {
                                ui.label(egui::RichText::new(n.icon).font(theme::regular(14.0)).color(t.accent));
                                ui.label(egui::RichText::new(&n.title).font(theme::semibold(12.5)));
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    ui.label(egui::RichText::new(when).color(t.text3).size(11.5));
                                });
                            });
                            if !n.body.is_empty() {
                                ui.label(egui::RichText::new(&n.body).color(t.text2).size(12.0));
                            }
                        });
                    let resp = r.response.interact(Sense::click());
                    if resp.hovered() {
                        ui.painter().rect_filled(r.response.rect, 6, t.hover_wash());
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

//! Modal dialogs: symbol search, Indicators, an indicator's settings
//! (Inputs / Style / Visibility), Chart settings, and naming a template.

mod alert;
mod chart_settings;
pub mod color;
mod indicators;
pub mod providers;
mod study;
mod symbol;

use crate::app::App;
use crate::theme::{self, Theme};
use egui::{Align, CornerRadius, Id, Layout, Margin, Ui, vec2};

pub use alert::AlertForm;
pub use chart_settings::ChartSettings;
pub use indicators::Indicators;
pub use study::StudySettings;
pub use symbol::SymbolSearch;

pub enum Modal {
    Symbol(SymbolSearch),
    Indicators(Indicators),
    Study(StudySettings),
    ChartSettings(ChartSettings),
    SaveTemplate {
        kind: TemplateKind,
        name: String,
    },
    /// "Change interval": opened by typing a number.
    Interval(String),
    Alert(Box<AlertForm>),
    Providers(Box<providers::Providers>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TemplateKind {
    Chart,
    Indicators,
}

impl TemplateKind {
    pub fn key(self) -> &'static str {
        match self {
            TemplateKind::Chart => "chart",
            TemplateKind::Indicators => "indicators",
        }
    }
}

/// What a dialog asks for when it's done with this frame.
pub enum Done {
    Keep,
    Close,
}

/// The frame every dialog sits in: elevated card, rounded, with a title
/// row and a close button.
pub fn frame(ui: &mut Ui, t: &Theme, title: &str, width: f32, body: impl FnOnce(&mut Ui)) -> bool {
    let mut close = false;
    ui.set_width(width);
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(title).font(theme::semibold(16.0)));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if crate::widgets::tool_button(ui, t, crate::icons::ph::X, "Close (Esc)", false, 28.0).clicked() {
                close = true;
            }
        });
    });
    ui.add_space(8.0);
    body(ui);
    close
}

impl App {
    /// Draws the open dialog, if any.
    pub(crate) fn modal(&mut self, ctx: &egui::Context) {
        let Some(mut modal) = self.modal.take() else { return };
        let t = self.t();
        let resp = egui::Modal::new(Id::new("modal"))
            .backdrop_color(egui::Color32::from_black_alpha(if t.dark { 110 } else { 60 }))
            .frame(
                egui::Frame::new()
                    .fill(t.elevated)
                    .stroke(egui::Stroke::new(1.0, t.border))
                    .corner_radius(CornerRadius::same(theme::RADIUS))
                    .inner_margin(Margin::same(18))
                    .shadow(egui::Shadow {
                        offset: [0, 12],
                        blur: 36,
                        spread: 0,
                        color: t.shadow(),
                    }),
            )
            .show(ctx, |ui| match &mut modal {
                Modal::Symbol(s) => s.show(ui, self, &t),
                Modal::Indicators(s) => s.show(ui, self, &t),
                Modal::Study(s) => s.show(ui, self, &t),
                Modal::ChartSettings(s) => s.show(ui, self, &t),
                Modal::SaveTemplate { kind, name } => save_template(ui, self, &t, *kind, name),
                Modal::Interval(text) => interval(ui, self, &t, text),
                Modal::Alert(f) => f.show(ui, self, &t),
                Modal::Providers(p) => p.show(ui, self, &t),
            });
        let done = matches!(resp.inner, Done::Close);
        if done {
            return;
        }
        if resp.should_close() {
            // Esc or a click outside: settings dialogs cancel their preview.
            match &mut modal {
                Modal::Study(s) => s.cancel(self),
                Modal::ChartSettings(s) => s.cancel(self),
                _ => {}
            }
            return;
        }
        // A dialog may have opened another one (e.g. Save template).
        if self.modal.is_none() {
            self.modal = Some(modal);
        }
    }
}

fn save_template(ui: &mut Ui, app: &mut App, t: &Theme, kind: TemplateKind, name: &mut String) -> Done {
    let title = match kind {
        TemplateKind::Chart => "Save chart template",
        TemplateKind::Indicators => "Save indicator template",
    };
    let mut done = Done::Keep;
    if frame(ui, t, title, 360.0, |ui| {
        ui.label(egui::RichText::new("Template name").color(t.text2));
        let r = ui.add(egui::TextEdit::singleline(name).desired_width(f32::INFINITY));
        if !r.has_focus() && name.is_empty() {
            r.request_focus();
        }
        let exists = app.store.templates(kind.key()).iter().any(|x| x.name == name.trim());
        if exists {
            ui.label(egui::RichText::new("A template with this name exists; saving replaces it.").color(t.text3));
        }
        ui.add_space(10.0);
        // A row, not a right-to-left layout: that one takes all the height a
        // modal offers and stretched this dialog down the screen.
        ui.horizontal(|ui| {
            let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            ui.add_space(ui.available_width() - 150.0);
            if ui.button("Cancel").clicked() {
                done = Done::Close;
            }
            let ok = ui
                .add_enabled(!name.trim().is_empty(), egui::Button::new("Save"))
                .clicked();
            if (ok || enter) && !name.trim().is_empty() {
                app.save_template(kind, name.trim());
                done = Done::Close;
            }
        });
    }) {
        done = Done::Close;
    }
    done
}

fn interval(ui: &mut Ui, app: &mut App, t: &Theme, text: &mut String) -> Done {
    let mut done = Done::Keep;
    if frame(ui, t, "Change interval", 300.0, |ui| {
        let r = ui.add(
            egui::TextEdit::singleline(text)
                .font(theme::regular(15.0))
                .desired_width(f32::INFINITY),
        );
        if !r.has_focus() {
            r.request_focus();
            if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), r.id) {
                let end = egui::text::CCursor::new(text.chars().count());
                state.cursor.set_char_range(Some(egui::text::CCursorRange::one(end)));
                state.store(ui.ctx(), r.id);
            }
        }
        let parsed = opentern_core::Timeframe::parse(text);
        let hint = match parsed {
            Some(tf) => format!("Press Enter for {}", tf.label()),
            None => "Like 15, 4h, D, W or 3M".into(),
        };
        ui.label(egui::RichText::new(hint).color(t.text3));
        if ui.input(|i| i.key_pressed(egui::Key::Enter))
            && let Some(tf) = parsed
        {
            app.set_interval_synced(tf);
            done = Done::Close;
        }
    }) {
        done = Done::Close;
    }
    done
}

/// A labelled row in a settings form: label on the left, control on the
/// right, lined up in two columns.
pub fn form_row(ui: &mut Ui, t: &Theme, label: &str, control: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(vec2(170.0, 26.0), egui::Sense::hover());
        ui.painter().text(
            r.left_center(),
            egui::Align2::LEFT_CENTER,
            label,
            theme::regular(13.0),
            t.text2,
        );
        control(ui);
    });
}

pub fn section(ui: &mut Ui, t: &Theme, title: &str) {
    ui.add_space(6.0);
    ui.label(
        egui::RichText::new(title.to_uppercase())
            .font(theme::semibold(11.0))
            .color(t.text3),
    );
    ui.add_space(2.0);
}

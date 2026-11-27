//! Modal dialogs: symbol search, Indicators, an indicator's settings
//! (Inputs / Style / Visibility), Chart settings, and naming a template.

use crate::i18n::{tr, trf};
mod alert;
mod chart_settings;
pub mod color;
mod indicators;
pub mod paper_account;
pub mod providers;
pub mod strategy_props;
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
    StrategyProps(Box<strategy_props::StrategyProps>),
    /// A paper account's name, starting capital and commission (new, or
    /// the one in use).
    PaperAccount(Box<paper_account::PaperAccountForm>),
    WhatsNew(Box<crate::whatsnew::Card>),
    Index(Box<crate::indices::IndexEditor>),
    /// "Clear the price cache?" (Settings → General).
    ClearCache,
    /// The first Python script in the scripts folder: what Python can do.
    PythonNotice,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TemplateKind {
    Chart,
    Indicators,
    /// A drawing tool's named styles. Kept apart from the plain `drawing`
    /// kind, which holds each tool's style for new drawings (the star).
    Drawing(crate::chart::draw::Tool),
}

impl TemplateKind {
    pub fn key(self) -> String {
        match self {
            TemplateKind::Chart => "chart".into(),
            TemplateKind::Indicators => "indicators".into(),
            TemplateKind::Drawing(tool) => format!("drawing:{}", tool.key()),
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
        ui.label(egui::RichText::new(title).font(theme::semibold(17.0)));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if crate::widgets::tool_button(ui, t, crate::icons::ph::X, "", false, 28.0).clicked() {
                close = true;
            }
        });
    });
    // A hairline under the title, edge to edge across the dialog.
    ui.add_space(10.0);
    let (r, _) = ui.allocate_exact_size(vec2(width, 1.0), egui::Sense::hover());
    let edge = (r.left() - MARGIN as f32)..=(r.right() + MARGIN as f32);
    ui.painter().hline(edge, r.center().y, egui::Stroke::new(1.0, t.border));
    ui.add_space(14.0);
    body(ui);
    close
}

/// A dialog's padding.
const MARGIN: i8 = 22;

/// The border of the big dialogs: firmer than menus', since they stand
/// alone mid-screen.
fn border_stroke(t: &Theme) -> egui::Stroke {
    egui::Stroke::new(1.5, t.edge().lerp_to_gamma(t.text, if t.dark { 0.10 } else { 0.06 }))
}

/// A glow of the app's green falling on a dialog from above, fading into
/// the surface over its top. Drawn as stacked rounded bands (so corners
/// stay round) in exact, solid colours: faint see-through green on the
/// slate rounded to equal parts green and blue and read as teal. The glow
/// is the accent darkened (like the selected rows' fill), mixed in just
/// far enough that it reads green, not teal, on the bluish surface.
/// Call [`sheen_slots`] before the dialog's content and this after it, so
/// it spans the finished dialog yet sits behind the content.
pub fn sheen(ui: &Ui, t: &Theme, slots: &[egui::layers::ShapeIdx], frame: egui::Rect, radius: u8) {
    let p = ui.painter();
    let reach = 120.0_f32.min(frame.height());
    let top = CornerRadius {
        nw: radius,
        ne: radius,
        sw: 0,
        se: 0,
    };
    let base = t.popup();
    let (glow, most) = if t.dark {
        {
            let a = t.accent;
            let q = |c: u8| (c as f32 * 0.25).round() as u8;
            (egui::Color32::from_rgb(q(a.r()), q(a.g()), q(a.b())), 0.42)
        }
    } else {
        (t.accent, 0.07)
    };
    let n = slots.len();
    for (k, slot) in slots.iter().enumerate() {
        // Tallest and faintest first, the short strong ones on top.
        let h = reach * (n - k) as f32 / n as f32;
        let c = base.lerp_to_gamma(glow, most * (k + 1) as f32 / n as f32);
        // At least twice the radius: egui caps a corner at half the
        // rect's height, and a sharper corner poked past the border's curve.
        let r = egui::Rect::from_min_size(frame.min, vec2(frame.width(), h.max(2.0 * radius as f32 + 1.0)));
        p.set(*slot, egui::Shape::rect_filled(r, top, c));
    }
}

/// Places for [`sheen`]'s bands, behind what's drawn next.
pub fn sheen_slots(ui: &Ui) -> Vec<egui::layers::ShapeIdx> {
    // 24 bands, then the border drawn again over them.
    (0..25).map(|_| ui.painter().add(egui::Shape::Noop)).collect()
}

impl App {
    /// Draws the open dialog, if any.
    pub(crate) fn modal(&mut self, ctx: &egui::Context) {
        let Some(mut modal) = self.modal.take() else { return };
        let t = self.t();
        let resp = egui::Modal::new(Id::new("modal"))
            .backdrop_color(egui::Color32::from_black_alpha(if t.dark { 150 } else { 70 }))
            .frame(
                egui::Frame::new()
                    .fill(t.popup())
                    .stroke(border_stroke(&t))
                    .corner_radius(CornerRadius::same(16))
                    .inner_margin(Margin::same(MARGIN))
                    .shadow(egui::Shadow {
                        offset: [0, 24],
                        blur: 64,
                        spread: 0,
                        color: t.shadow(),
                    }),
            )
            .show(ctx, |ui| {
                let slots = sheen_slots(ui);
                let done = match &mut modal {
                    Modal::Symbol(s) => s.show(ui, self, &t),
                    Modal::Indicators(s) => s.show(ui, self, &t),
                    Modal::Study(s) => s.show(ui, self, &t),
                    Modal::ChartSettings(s) => s.show(ui, self, &t),
                    Modal::SaveTemplate { kind, name } => save_template(ui, self, &t, *kind, name),
                    Modal::Interval(text) => interval(ui, self, &t, text),
                    Modal::Alert(f) => f.show(ui, self, &t),
                    Modal::Providers(p) => p.show(ui, self, &t),
                    Modal::StrategyProps(p) => p.show(ui, self, &t),
                    Modal::PaperAccount(f) => f.show(ui, self, &t),
                    Modal::Index(e) => e.show(ui, self, &t),
                    Modal::ClearCache => clear_cache(ui, self, &t),
                    Modal::PythonNotice => python_notice(ui, &t),
                    Modal::WhatsNew(c) => {
                        if crate::whatsnew::card(c, ui, &t) {
                            Done::Close
                        } else {
                            Done::Keep
                        }
                    }
                };
                // Inside the border: the frame's corner less its stroke.
                let fill = ui.min_rect().expand(MARGIN as f32);
                sheen(ui, &t, &slots[..slots.len() - 1], fill, 15);
                // The border once more over the glow (as egui draws it:
                // inside the frame's outer edge), so the glow's corners can
                // never cover it, at any screen scale.
                let border = border_stroke(&t);
                ui.painter().set(
                    slots[slots.len() - 1],
                    egui::Shape::Rect(egui::epaint::RectShape::new(
                        fill.expand(border.width),
                        CornerRadius::same(16),
                        egui::Color32::TRANSPARENT,
                        border,
                        egui::StrokeKind::Inside,
                    )),
                );
                done
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
        TemplateKind::Chart => tr("Save chart template"),
        TemplateKind::Indicators => tr("Save indicator template"),
        TemplateKind::Drawing(_) => tr("Save drawing template"),
    };
    let mut done = Done::Keep;
    if frame(ui, t, title, 380.0, |ui| {
        if kind == TemplateKind::Indicators && !app.chart.studies.is_empty() {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
                for s in &app.chart.studies {
                    let name = s.meta().short;
                    egui::Frame::new()
                        .fill(t.hover)
                        .corner_radius(CornerRadius::same(6))
                        .inner_margin(Margin::symmetric(8, 3))
                        .show(ui, |ui| {
                            ui.label(egui::RichText::new(name).font(theme::medium(12.0)).color(t.text2));
                        });
                }
            });
        }
        ui.add_space(10.0);
        ui.label(egui::RichText::new(tr("Name")).font(theme::medium(12.0)).color(t.text2));
        let r = ui.add(
            crate::widgets::field(name)
                .hint_text(tr("e.g. Swing setup"))
                .desired_width(f32::INFINITY),
        );
        if !r.has_focus() && name.is_empty() {
            r.request_focus();
        }
        let exists = app.store.templates(&kind.key()).iter().any(|x| x.name == name.trim());
        if exists {
            ui.label(
                egui::RichText::new(trf!(
                    "{}  Saving replaces the template with this name.",
                    crate::icons::ph::INFO
                ))
                .font(theme::regular(12.0))
                .color(t.warn),
            );
        }
        ui.add_space(14.0);
        // A row, not a right-to-left layout: that one takes all the height a
        // modal offers and stretched this dialog down the screen.
        ui.horizontal(|ui| {
            let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            ui.add_space((ui.available_width() - 168.0).max(0.0));
            if crate::widgets::secondary(ui, tr("Cancel")).clicked() {
                done = Done::Close;
            }
            let ok = crate::widgets::button(
                ui,
                crate::widgets::Kind::Primary,
                !name.trim().is_empty(),
                None,
                tr("Save"),
            )
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
    if frame(ui, t, tr("Change interval"), 300.0, |ui| {
        let r = ui.add(
            crate::widgets::field(text)
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
            Some(tf) => trf!("Press Enter for {}", tf.label()),
            None => tr("Like 15, 4h, D, W or 3M").into(),
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
    form_row_w(ui, t, 170.0, label, control);
}

/// [`form_row`] with a label column `w` wide.
pub fn form_row_w(ui: &mut Ui, t: &Theme, w: f32, label: &str, control: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        // As tall as a field or a dropdown, so the label sits on the
        // control's centre line.
        let (r, _) = ui.allocate_exact_size(vec2(w, 34.0), egui::Sense::hover());
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

/// A section title with a "Beta" pill: for providers written from their
/// documentation that nobody has yet run against a real account.
pub fn section_beta(ui: &mut Ui, t: &Theme, title: &str) {
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(title.to_uppercase())
                .font(theme::semibold(11.0))
                .color(t.text3),
        );
        let text = egui::RichText::new(tr("Beta")).font(theme::medium(10.5)).color(t.warn);
        egui::Frame::new()
            .inner_margin(egui::Margin::symmetric(6, 1))
            .corner_radius(egui::CornerRadius::same(8))
            .stroke(egui::Stroke::new(1.0, t.warn.gamma_multiply(0.5)))
            .show(ui, |ui| ui.label(text))
            .response
            .on_hover_text(tr(
                "Not yet tried with a real account. Compare its prices with another source before relying on them.",
            ));
    });
    ui.add_space(2.0);
}

pub fn section(ui: &mut Ui, t: &Theme, title: &str) {
    ui.add_space(10.0);
    ui.label(egui::RichText::new(title).font(theme::semibold(13.0)).color(t.text));
    ui.add_space(2.0);
}

/// Asks before clearing the price cache: some of what it holds can't be
/// downloaded again (sources keep only recent minute and hour bars, and
/// Yahoo never has the special sessions built from live prices).
fn python_notice(ui: &mut Ui, t: &Theme) -> Done {
    let mut done = Done::Keep;
    if frame(ui, t, tr("Python scripts run on your computer"), 420.0, |ui| {
        ui.label(
            egui::RichText::new(tr(
                "A Python script runs in your own system's Python. Based on the code, it can read and \
                 change your files and go online, and that includes code an agent writes for you. Only \
                 run the ones you trust.",
            ))
            .color(t.text2),
        );
        ui.add_space(4.0);
        ui.label(
            egui::RichText::new(tr(
                "Pine, Luau and JavaScript scripts stay sealed inside Opentern. They are safer alternatives.",
            ))
            .color(t.text2),
        );
        ui.add_space(16.0);
        // A fixed-size row: a bare right-to-left layout takes the whole
        // modal's height.
        ui.allocate_ui_with_layout(
            vec2(ui.available_width(), 32.0),
            Layout::right_to_left(Align::Center),
            |ui| {
                if crate::widgets::primary(ui, tr("Got it")).clicked() {
                    done = Done::Close;
                }
            },
        );
    }) {
        done = Done::Close;
    }
    done
}

fn clear_cache(ui: &mut Ui, app: &mut App, t: &Theme) -> Done {
    let mut done = Done::Keep;
    if frame(ui, t, tr("Clear the price cache?"), 400.0, |ui| {
        ui.label(
            egui::RichText::new(tr(
                "Charts download their prices again when you open them. Older minute and hour bars may \
                 not come back: data sources only keep recent ones (Yahoo: about a week of 1-minute bars, \
                 60 days of 15-minute). Muhurat and other special sessions recorded live won't come back \
                 either.",
            ))
            .color(t.text2),
        );
        ui.add_space(4.0);
        ui.label(
            egui::RichText::new(tr("Your watchlists, layouts, drawings, alerts and scripts stay.")).color(t.text2),
        );
        ui.add_space(16.0);
        // A fixed-size row: a bare right-to-left layout takes the whole
        // modal's height.
        ui.allocate_ui_with_layout(
            vec2(ui.available_width(), 32.0),
            Layout::right_to_left(Align::Center),
            |ui| {
                if crate::widgets::button(ui, crate::widgets::Kind::Danger, true, None, tr("Clear cache")).clicked() {
                    app.clear_price_cache(ui.ctx());
                    done = Done::Close;
                }
                if crate::widgets::secondary(ui, tr("Cancel")).clicked() {
                    done = Done::Close;
                }
            },
        );
    }) {
        done = Done::Close;
    }
    done
}

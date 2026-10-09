//! Create / edit an alert: the condition (the price, a study's plot, a
//! script's alert, or a drawn line), how often it fires, when it expires,
//! the message, and where it's sent.

use super::{Done, form_row, frame, section};
use crate::alerts::{KEYCHAIN_SERVICE, TELEGRAM_CHAT, TELEGRAM_TOKEN, line_condition, now_ms};
use crate::app::App;
use crate::theme::Theme;
use egui::Ui;
use opentern_engine::alerts::{Alert, Condition, Freq, Op};
use opentern_series::study::PlotStyle;

/// Something an alert can watch.
#[derive(Clone)]
struct Source {
    label: String,
    /// The condition with a placeholder operator/value.
    cond: Condition,
}

pub struct AlertForm {
    alert: Alert,
    sources: Vec<Source>,
    sel: usize,
    op: Op,
    value: f64,
    /// Plot alerts: compare with the price instead of a value.
    vs_price: bool,
    expire: bool,
    expire_days: u32,
    token: String,
    chat: String,
    has_token: bool,
    error: String,
}

impl App {
    /// Opens the alert dialog for a new alert on the chart (or to edit one).
    pub(crate) fn open_alert_dialog(&mut self, edit: Option<Alert>) {
        let price = self.last_price(&self.chart.symbol.id).unwrap_or(0.0);
        let mut sources = vec![Source {
            label: format!("Price: {}", self.chart.symbol.ticker()),
            cond: Condition::Price {
                op: Op::Crossing,
                value: price,
            },
        }];
        for s in &self.chart.studies {
            let m = s.meta();
            for (k, p) in m.plots.iter().enumerate() {
                if matches!(p.style, PlotStyle::Hidden | PlotStyle::Shape) {
                    continue;
                }
                sources.push(Source {
                    label: format!("{}: {}", m.short, p.title),
                    cond: Condition::Plot {
                        uid: s.uid,
                        study: m.short.to_string(),
                        plot: k,
                        title: format!("{} {}", m.short, p.title),
                        op: Op::Crossing,
                        value: None,
                    },
                });
            }
            let run = self.scripts.iter().find(|r| r.uid == Some(s.uid));
            if let Some(script) = run.and_then(|r| r.pine.as_ref()) {
                for title in script.alert_conditions() {
                    sources.push(Source {
                        label: format!("{}: {title}", m.short),
                        cond: Condition::Script {
                            uid: s.uid,
                            study: m.short.to_string(),
                            title: Some(title),
                        },
                    });
                }
                sources.push(Source {
                    label: format!("{}: any alert() call", m.short),
                    cond: Condition::Script {
                        uid: s.uid,
                        study: m.short.to_string(),
                        title: None,
                    },
                });
            }
        }
        for d in &self.chart.drawings {
            if let Some(cond) = line_condition(d, Op::Crossing) {
                sources.push(Source {
                    label: format!("Drawing: {} #{}", d.tool.name(), d.id),
                    cond,
                });
            }
        }
        let has_token = keyring::Entry::new(KEYCHAIN_SERVICE, TELEGRAM_TOKEN)
            .and_then(|e| e.get_password())
            .is_ok_and(|t| !t.is_empty());
        let chat = self.store.value(TELEGRAM_CHAT).unwrap_or_default();
        let mut form = AlertForm {
            alert: Alert {
                symbol: self.chart.symbol.id.clone(),
                tf: self.chart.tf.label(),
                ..Alert::default()
            },
            sources,
            sel: 0,
            op: Op::Crossing,
            value: price,
            vs_price: false,
            expire: false,
            expire_days: 30,
            token: String::new(),
            chat,
            has_token,
            error: String::new(),
        };
        if let Some(a) = edit {
            // Find the matching source, or add the alert's own.
            let k = form.sources.iter().position(|s| same_source(&s.cond, &a.condition));
            form.sel = match k {
                Some(k) => k,
                None => {
                    form.sources.push(Source {
                        label: a.condition.summary(),
                        cond: a.condition.clone(),
                    });
                    form.sources.len() - 1
                }
            };
            match &a.condition {
                Condition::Price { op, value } => {
                    form.op = *op;
                    form.value = *value;
                }
                Condition::Line { op, .. } => form.op = *op,
                Condition::Plot { op, value, .. } => {
                    form.op = *op;
                    form.vs_price = value.is_none();
                    form.value = value.unwrap_or(price);
                }
                Condition::Script { .. } => {}
            }
            form.expire = a.expires.is_some();
            form.alert = a;
        }
        self.modal = Some(super::Modal::Alert(Box::new(form)));
    }
}

fn same_source(a: &Condition, b: &Condition) -> bool {
    match (a, b) {
        (Condition::Price { .. }, Condition::Price { .. }) => true,
        (Condition::Plot { uid: u1, plot: p1, .. }, Condition::Plot { uid: u2, plot: p2, .. }) => u1 == u2 && p1 == p2,
        (Condition::Script { uid: u1, title: t1, .. }, Condition::Script { uid: u2, title: t2, .. }) => {
            u1 == u2 && t1 == t2
        }
        (Condition::Line { drawing: d1, .. }, Condition::Line { drawing: d2, .. }) => d1.is_some() && d1 == d2,
        _ => false,
    }
}

impl AlertForm {
    pub fn show(&mut self, ui: &mut Ui, app: &mut App, t: &Theme) -> Done {
        let editing = self.alert.id != 0;
        let mut done = Done::Keep;
        let title = if editing { "Edit alert" } else { "Create alert" };
        // Room for the buttons; the form scrolls on short windows.
        let max_h = (ui.ctx().content_rect().height() - 190.0).max(200.0);
        let close = frame(ui, t, title, 460.0, |ui| {
            egui::ScrollArea::vertical()
                .max_height(max_h)
                .show(ui, |ui| self.form(ui, app, t));
            self.buttons(ui, app, editing, &mut done);
        });
        if close {
            done = Done::Close;
        }
        done
    }

    fn form(&mut self, ui: &mut Ui, app: &mut App, t: &Theme) {
        {
            section(ui, t, "Condition");
            let labels: Vec<String> = self.sources.iter().map(|s| s.label.clone()).collect();
            form_row(ui, t, "Watch", |ui| {
                egui::ComboBox::from_id_salt("alert-source")
                    .width(250.0)
                    .selected_text(labels.get(self.sel).cloned().unwrap_or_default())
                    .show_ui(ui, |ui| {
                        for (k, l) in labels.iter().enumerate() {
                            ui.selectable_value(&mut self.sel, k, l);
                        }
                    });
            });
            let src = self.sources[self.sel].cond.clone();
            if !matches!(src, Condition::Script { .. }) {
                form_row(ui, t, "When it's", |ui| {
                    egui::ComboBox::from_id_salt("alert-op")
                        .width(250.0)
                        .selected_text(self.op.name())
                        .show_ui(ui, |ui| {
                            for o in Op::ALL {
                                ui.selectable_value(&mut self.op, o, o.name());
                            }
                        });
                });
            }
            match &src {
                Condition::Price { .. } => {
                    form_row(ui, t, "Value", |ui| {
                        ui.add(egui::DragValue::new(&mut self.value).speed(0.01).max_decimals(8));
                    });
                }
                Condition::Plot { .. } => {
                    form_row(ui, t, "Compared with", |ui| {
                        ui.radio_value(&mut self.vs_price, true, "Price");
                        ui.radio_value(&mut self.vs_price, false, "Value");
                        if !self.vs_price {
                            ui.add(egui::DragValue::new(&mut self.value).speed(0.01).max_decimals(8));
                        }
                    });
                }
                Condition::Line { .. } => {
                    form_row(ui, t, "Compared with", |ui| {
                        ui.label("the price, against the line");
                    });
                }
                Condition::Script { .. } => {
                    form_row(ui, t, "", |ui| {
                        ui.label(egui::RichText::new("Fires when the script raises it.").color(t.text2));
                    });
                }
            }
            form_row(ui, t, "Trigger", |ui| {
                egui::ComboBox::from_id_salt("alert-freq")
                    .width(250.0)
                    .selected_text(self.alert.freq.name())
                    .show_ui(ui, |ui| {
                        for f in Freq::ALL {
                            ui.selectable_value(&mut self.alert.freq, f, f.name());
                        }
                    });
            });
            form_row(ui, t, "Expires", |ui| {
                ui.checkbox(&mut self.expire, "");
                if self.expire {
                    ui.add(
                        egui::DragValue::new(&mut self.expire_days)
                            .range(1..=365)
                            .suffix(" days"),
                    );
                } else {
                    ui.label(egui::RichText::new("Never").color(t.text2));
                }
            });
            section(ui, t, "Message");
            form_row(ui, t, "Name", |ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.alert.name)
                        .hint_text(app.chart.symbol.ticker())
                        .desired_width(250.0),
                );
            });
            form_row(ui, t, "Message", |ui| {
                ui.add(
                    egui::TextEdit::multiline(&mut self.alert.message)
                        .hint_text("e.g. {{ticker}} crossed at {{close}}")
                        .desired_rows(2)
                        .desired_width(250.0),
                );
            });
            section(ui, t, "Notify me");
            form_row(ui, t, "Desktop", |ui| {
                ui.checkbox(&mut self.alert.notify, "Notification");
                ui.checkbox(&mut self.alert.sound, "Sound");
            });
            form_row(ui, t, "Webhook URL", |ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.alert.webhook)
                        .hint_text("https://…")
                        .desired_width(250.0),
                );
            });
            form_row(ui, t, "Telegram", |ui| {
                ui.checkbox(&mut self.alert.telegram, "");
            });
            if self.alert.telegram {
                form_row(ui, t, "Bot token", |ui| {
                    let hint = if self.has_token {
                        "Saved in your keychain (paste to replace)"
                    } else {
                        "From @BotFather"
                    };
                    ui.add(
                        egui::TextEdit::singleline(&mut self.token)
                            .password(true)
                            .hint_text(hint)
                            .desired_width(250.0),
                    );
                });
                form_row(ui, t, "Chat id", |ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.chat)
                            .hint_text("Your chat id (e.g. from @userinfobot)")
                            .desired_width(250.0),
                    );
                });
            }
        }
    }

    fn buttons(&mut self, ui: &mut Ui, app: &mut App, editing: bool, done: &mut Done) {
        let t = app.t();
        let src = self.sources[self.sel].cond.clone();
        {
            if !self.error.is_empty() {
                ui.add_space(4.0);
                ui.label(egui::RichText::new(&self.error).color(t.neg));
            }
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if editing && ui.button("Delete").clicked() {
                    app.delete_alert(self.alert.id);
                    *done = Done::Close;
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let label = if editing { "  Save  " } else { "  Create  " };
                    if ui.button(label).clicked() {
                        match self.build(&src) {
                            Ok(a) => {
                                if self.alert.telegram
                                    && let Err(e) = self.save_telegram(app)
                                {
                                    self.error = e;
                                    return;
                                }
                                app.save_alert(a);
                                *done = Done::Close;
                            }
                            Err(e) => self.error = e,
                        }
                    }
                    if ui.button("Cancel").clicked() {
                        *done = Done::Close;
                    }
                });
            });
        }
    }

    fn build(&self, src: &Condition) -> Result<Alert, String> {
        let mut a = self.alert.clone();
        a.condition = match src.clone() {
            Condition::Price { .. } => {
                if !(self.value.is_finite()) {
                    return Err("Enter a price.".into());
                }
                Condition::Price {
                    op: self.op,
                    value: self.value,
                }
            }
            Condition::Line {
                t1,
                p1,
                t2,
                p2,
                extend,
                drawing,
                ..
            } => Condition::Line {
                op: self.op,
                t1,
                p1,
                t2,
                p2,
                extend,
                drawing,
            },
            Condition::Plot {
                uid,
                study,
                plot,
                title,
                ..
            } => Condition::Plot {
                uid,
                study,
                plot,
                title,
                op: self.op,
                value: (!self.vs_price).then_some(self.value),
            },
            s @ Condition::Script { .. } => s,
        };
        a.expires = self.expire.then(|| now_ms() + self.expire_days as i64 * 86_400_000);
        if !a.webhook.trim().is_empty() && !a.webhook.trim().starts_with("http") {
            return Err("The webhook URL should start with https://".into());
        }
        a.active = true;
        Ok(a)
    }

    /// Keeps the bot token in the OS keychain and the chat id in settings.
    fn save_telegram(&mut self, app: &mut App) -> Result<(), String> {
        let token = self.token.trim();
        if !token.is_empty() {
            keyring::Entry::new(KEYCHAIN_SERVICE, TELEGRAM_TOKEN)
                .and_then(|e| e.set_password(token))
                .map_err(|e| format!("Couldn't save the token in your keychain: {e}"))?;
            self.has_token = true;
            self.token.clear();
        } else if !self.has_token {
            return Err("Paste your Telegram bot token, or turn Telegram off.".into());
        }
        if self.chat.trim().is_empty() {
            return Err("Enter the Telegram chat id to send alerts to.".into());
        }
        app.store
            .set_value(TELEGRAM_CHAT, self.chat.trim())
            .map_err(|e| format!("Couldn't save the chat id: {e}"))
    }
}

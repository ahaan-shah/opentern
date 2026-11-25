//! Create / edit an alert: the condition (the price, a study's plot, a
//! script's alert, or a drawn line), how often it fires, when it expires,
//! the message, and where it's sent.

use super::{Done, frame};
use crate::alerts::{TELEGRAM_CHAT, TELEGRAM_TOKEN, line_condition, now_ms};
use crate::app::App;
use crate::i18n::{msg, nt, tr, trl, trm};
use crate::theme::Theme;
use egui::{Ui, vec2};
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
    /// Email: server, port, login, password (to the keychain), recipient.
    email: [String; 4],
    email_password: String,
    has_password: bool,
    /// The test email's outcome, filled in by its thread.
    email_status: std::sync::Arc<std::sync::Mutex<String>>,
    error: String,
    /// The value as typed (None until the form first shows it).
    value_text: Option<String>,
    /// The webhook chip (None until the form first shows: on when a URL is set).
    webhook_on: Option<bool>,
}

impl App {
    /// Opens the alert dialog for a new alert on the chart (or to edit one).
    pub(crate) fn open_alert_dialog(&mut self, edit: Option<Alert>) {
        let price = self.last_price(&self.chart.symbol.id).unwrap_or(0.0);
        let mut sources = vec![Source {
            label: msg!("Price: {}", self.chart.symbol.ticker()),
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
                    label: msg!("{}: any alert() call", m.short),
                    cond: Condition::Script {
                        uid: s.uid,
                        study: m.short.to_string(),
                        title: None,
                    },
                });
            } else if run.is_some_and(|r| r.lang.in_process()) {
                // Luau and JavaScript have alert() but no alertcondition.
                sources.push(Source {
                    label: msg!("{}: any alert() call", m.short),
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
                    label: msg!("Drawing: {} #{}", d.tool.name(), d.id),
                    cond,
                });
            }
        }
        let has_token = opentern_core::secrets::get(TELEGRAM_TOKEN).is_some();
        let chat = self.store.value(TELEGRAM_CHAT).unwrap_or_default();
        let has_password = opentern_core::secrets::get(crate::alerts::EMAIL_PASSWORD).is_some();
        let v = |k: &str| self.store.value(k).unwrap_or_default();
        let email = [
            v(crate::alerts::EMAIL_HOST),
            match v(crate::alerts::EMAIL_PORT) {
                p if p.is_empty() => "587".into(),
                p => p,
            },
            v(crate::alerts::EMAIL_USER),
            v(crate::alerts::EMAIL_TO),
        ];
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
            email,
            email_password: String::new(),
            has_password,
            email_status: Default::default(),
            error: String::new(),
            value_text: None,
            webhook_on: None,
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

/// A price as typed: no trailing zeros ("164.595", "68270.49").
fn number_text(v: f64) -> String {
    if !v.is_finite() {
        return String::new();
    }
    let s = format!("{v:.8}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

impl AlertForm {
    pub fn show(&mut self, ui: &mut Ui, app: &mut App, t: &Theme) -> Done {
        let editing = self.alert.id != 0;
        let mut done = Done::Keep;
        let title = if editing { tr("Edit alert") } else { tr("Create alert") };
        // Room for the buttons; the form scrolls on short windows.
        let max_h = (ui.ctx().content_rect().height() - 190.0).max(200.0);
        let close = frame(ui, t, title, 500.0, |ui| {
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
            use crate::widgets::{field, pair, stacked};
            let full = ui.available_width();
            let labels: Vec<String> = self.sources.iter().map(|s| trl(&s.label).into_owned()).collect();
            stacked(ui, t, tr("Watch"), full, |ui| {
                crate::widgets::combo("alert-source")
                    .width(full)
                    .selected_text(labels.get(self.sel).cloned().unwrap_or_default())
                    .show_ui(ui, |ui| {
                        for (k, l) in labels.iter().enumerate() {
                            crate::widgets::option_value(ui, &mut self.sel, k, l);
                        }
                    });
            });
            let src = self.sources[self.sel].cond.clone();
            let value_text = self.value_text.get_or_insert_with(|| number_text(self.value));
            let value = &mut self.value;
            let mut value_field = |ui: &mut Ui, w: f32| {
                let r = ui.add(field(value_text).desired_width(w).hint_text(tr("Price")));
                if r.changed()
                    && let Ok(v) = value_text.trim().replace(',', "").parse::<f64>()
                {
                    *value = v;
                }
            };
            let op = &mut self.op;
            let mut op_field = |ui: &mut Ui, w: f32| {
                crate::widgets::combo("alert-op")
                    .width(w)
                    .selected_text(tr(op.name()))
                    .show_ui(ui, |ui| {
                        for o in Op::ALL {
                            crate::widgets::option_value(ui, op, o, tr(o.name()));
                        }
                    });
            };
            match &src {
                Condition::Price { .. } => {
                    pair(ui, t, (tr("When it's"), &mut op_field), (tr("Value"), &mut value_field))
                }
                Condition::Plot { .. } => {
                    pair(
                        ui,
                        t,
                        (tr("When it's"), &mut op_field),
                        (tr("Compared with"), &mut |ui: &mut Ui, _w: f32| {
                            ui.horizontal(|ui| {
                                ui.set_min_height(32.0);
                                crate::widgets::radio(ui, &mut self.vs_price, true, tr("Price"));
                                crate::widgets::radio(ui, &mut self.vs_price, false, tr("A value"));
                            });
                        }),
                    );
                    if !self.vs_price {
                        stacked(ui, t, tr("Value"), full, |ui| value_field(ui, full));
                    }
                }
                Condition::Line { .. } => {
                    stacked(ui, t, tr("When the price is"), full, |ui| op_field(ui, full));
                }
                Condition::Script { .. } => {}
            }
            let freq = &mut self.alert.freq;
            let (expire, days) = (&mut self.expire, &mut self.expire_days);
            pair(
                ui,
                t,
                (tr("Trigger"), &mut |ui: &mut Ui, w: f32| {
                    crate::widgets::combo("alert-freq")
                        .width(w)
                        .selected_text(tr(freq.name()))
                        .show_ui(ui, |ui| {
                            for f in Freq::ALL {
                                crate::widgets::option_value(ui, freq, f, tr(f.name()));
                            }
                        });
                }),
                (tr("Expires"), &mut |ui: &mut Ui, _w: f32| {
                    ui.horizontal(|ui| {
                        ui.set_min_height(32.0);
                        crate::widgets::checkbox(ui, expire, if *expire { tr("After") } else { tr("Never") });
                        if *expire {
                            ui.add_sized([96.0, 32.0], egui::DragValue::new(days).range(1..=365).suffix(" days"));
                        }
                    });
                }),
            );
            crate::widgets::form_rule(ui, t);
            stacked(ui, t, tr("Name"), full, |ui| {
                ui.add(
                    field(&mut self.alert.name)
                        .hint_text(app.chart.symbol.ticker())
                        .desired_width(full),
                );
            });
            stacked(ui, t, tr("Message"), full, |ui| {
                ui.add(
                    crate::widgets::area(&mut self.alert.message)
                        .hint_text(tr("e.g. {{ticker}} crossed at {{close}}"))
                        .desired_rows(2)
                        .desired_width(full),
                );
            });
            crate::widgets::form_rule(ui, t);
            crate::widgets::field_label(ui, t, tr("Notify me"));
            let webhook_on = self.webhook_on.get_or_insert(!self.alert.webhook.trim().is_empty());
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
                use crate::icons::ph;
                crate::widgets::chip(ui, t, &mut self.alert.notify, ph::BELL, tr("Notification"));
                crate::widgets::chip(ui, t, &mut self.alert.sound, ph::SPEAKER_HIGH, tr("Sound"));
                crate::widgets::chip(ui, t, webhook_on, ph::WEBHOOKS_LOGO, tr("Webhook"));
                crate::widgets::chip(ui, t, &mut self.alert.telegram, ph::TELEGRAM_LOGO, tr("Telegram"));
                crate::widgets::chip(ui, t, &mut self.alert.email, ph::ENVELOPE_SIMPLE, tr("Email"));
            });
            if !*webhook_on {
                self.alert.webhook.clear();
            } else {
                ui.add_space(4.0);
                stacked(ui, t, tr("Webhook URL"), full, |ui| {
                    ui.add(
                        field(&mut self.alert.webhook)
                            .hint_text(tr("https://…"))
                            .desired_width(full),
                    );
                });
            }
            if self.alert.telegram {
                stacked(ui, t, tr("Telegram bot token"), full, |ui| {
                    let hint = if self.has_token {
                        tr("Saved in your keychain (paste to replace)")
                    } else {
                        tr("From @BotFather")
                    };
                    ui.add(
                        crate::widgets::field(&mut self.token)
                            .password(true)
                            .hint_text(hint)
                            .desired_width(full),
                    );
                });
                stacked(ui, t, tr("Telegram chat id"), full, |ui| {
                    ui.add(
                        crate::widgets::field(&mut self.chat)
                            .hint_text(tr("Your chat id (e.g. from @userinfobot)"))
                            .desired_width(full),
                    );
                });
            }
            if self.alert.email {
                let hints = [
                    (tr("SMTP server"), "smtp.gmail.com"),
                    (tr("Port"), "587"),
                    (tr("Your address"), "you@example.com"),
                    (tr("Send to"), "you@example.com"),
                ];
                for (k, (label, hint)) in hints.iter().enumerate() {
                    stacked(ui, t, label, full, |ui| {
                        ui.add(
                            crate::widgets::field(&mut self.email[k])
                                .hint_text(*hint)
                                .desired_width(full),
                        );
                    });
                }
                stacked(ui, t, tr("Password"), full, |ui| {
                    let hint = if self.has_password {
                        tr("Saved in your keychain (paste to replace)")
                    } else {
                        tr("An app password, not your login")
                    };
                    ui.add(
                        crate::widgets::field(&mut self.email_password)
                            .password(true)
                            .hint_text(hint)
                            .desired_width(full),
                    );
                });
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if crate::widgets::secondary(ui, tr("Send a test email")).clicked() {
                        let status = self.email_status.clone();
                        let set = move |s: String| {
                            if let Ok(mut m) = status.lock() {
                                *m = s;
                            }
                        };
                        match self.save_email(app) {
                            Ok(()) => match (
                                app.email_to(),
                                opentern_core::secrets::get(crate::alerts::EMAIL_PASSWORD),
                            ) {
                                (Some(to), Some(pw)) => {
                                    set(nt!("Sending…").into());
                                    let ctx = ui.ctx().clone();
                                    let _ = std::thread::Builder::new().name("test-email".into()).spawn(move || {
                                        let r =
                                            crate::alerts::send_email(&to, &pw, "Opentern test", "Email alerts work.");
                                        set(match r {
                                            Ok(()) => nt!("Sent. Check your inbox.").into(),
                                            Err(e) => e.replace(&pw, "…"),
                                        });
                                        ctx.request_repaint();
                                    });
                                }
                                _ => set(nt!("Fill in every field first.").into()),
                            },
                            Err(e) => set(e),
                        }
                    }
                    let status = self.email_status.lock().map(|s| s.clone()).unwrap_or_default();
                    if !status.is_empty() {
                        ui.label(egui::RichText::new(trm(&status)).color(t.text3).size(11.5));
                    }
                });
                ui.label(
                    egui::RichText::new(
                        tr("Gmail, Outlook and most providers need an app password: make one in your account's security settings."),
                    )
                    .color(t.text3)
                    .size(11.5),
                );
            }
        }
    }

    fn buttons(&mut self, ui: &mut Ui, app: &mut App, editing: bool, done: &mut Done) {
        let t = app.t();
        let src = self.sources[self.sel].cond.clone();
        {
            if !self.error.is_empty() {
                ui.add_space(4.0);
                ui.label(egui::RichText::new(trm(&self.error)).color(t.neg));
            }
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if editing
                    && crate::widgets::button(ui, crate::widgets::Kind::Danger, true, None, tr("Delete")).clicked()
                {
                    app.delete_alert(self.alert.id);
                    *done = Done::Close;
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let label = if editing { tr("Save") } else { tr("Create") };
                    if crate::widgets::primary(ui, &format!("  {label}  ")).clicked() {
                        match self.build(&src) {
                            Ok(a) => {
                                if self.alert.telegram
                                    && let Err(e) = self.save_telegram(app)
                                {
                                    self.error = e;
                                    return;
                                }
                                if self.alert.email
                                    && let Err(e) = self.save_email(app)
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
                    if crate::widgets::secondary(ui, tr("Cancel")).clicked() {
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
                    return Err(nt!("Enter a price.").into());
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
            return Err(nt!("The webhook URL should start with https://").into());
        }
        a.active = true;
        Ok(a)
    }

    /// Keeps the SMTP password in the OS keychain, the rest in settings.
    fn save_email(&mut self, app: &mut App) -> Result<(), String> {
        let [host, port, user, to] = self.email.clone().map(|s| s.trim().to_string());
        if host.is_empty() || user.is_empty() || to.is_empty() {
            return Err(nt!("Fill in the SMTP server, your address and where to send, or turn email off.").into());
        }
        if port.parse::<u16>().is_err() {
            return Err(nt!("The port should be a number, usually 587 or 465.").into());
        }
        let pw = self.email_password.trim();
        if !pw.is_empty() {
            opentern_core::secrets::set(crate::alerts::EMAIL_PASSWORD, pw)
                .map_err(|e| msg!("Couldn't save the password in your keychain: {e}"))?;
            self.has_password = true;
            self.email_password.clear();
        } else if !self.has_password {
            return Err(nt!("Paste the email password (an app password), or turn email off.").into());
        }
        for (k, v) in [
            (crate::alerts::EMAIL_HOST, host),
            (crate::alerts::EMAIL_PORT, port),
            (crate::alerts::EMAIL_USER, user),
            (crate::alerts::EMAIL_TO, to),
        ] {
            app.store
                .set_value(k, &v)
                .map_err(|e| msg!("Couldn't save the email settings: {e}"))?;
        }
        Ok(())
    }

    /// Keeps the bot token in the OS keychain and the chat id in settings.
    fn save_telegram(&mut self, app: &mut App) -> Result<(), String> {
        let token = self.token.trim();
        if !token.is_empty() {
            opentern_core::secrets::set(TELEGRAM_TOKEN, token)
                .map_err(|e| msg!("Couldn't save the token in your keychain: {e}"))?;
            self.has_token = true;
            self.token.clear();
        } else if !self.has_token {
            return Err(nt!("Paste your Telegram bot token, or turn Telegram off.").into());
        }
        if self.chat.trim().is_empty() {
            return Err(nt!("Enter the Telegram chat id to send alerts to.").into());
        }
        app.store
            .set_value(TELEGRAM_CHAT, self.chat.trim())
            .map_err(|e| msg!("Couldn't save the chat id: {e}"))
    }
}

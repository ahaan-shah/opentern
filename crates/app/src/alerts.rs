//! Alerts in the app: evaluated as prices arrive (whether or not the
//! symbol is on the chart, for price alerts), fired into the Alerts log, a
//! toast, and the channels the alert asks for. Desktop notifications,
//! webhooks, Telegram and email are sent from a background thread, so a
//! slow network never stalls a frame. The Telegram bot token and the email
//! password live in the OS keychain; the rest in settings.

use crate::app::App;
use crate::chart::draw::Tool;
use crate::shell::small_button;
use crate::theme::{self, Theme};
use crossbeam_channel::{Sender, unbounded};
use egui::{Align, Align2, Color32, CornerRadius, Layout, Sense, Stroke, Ui, pos2, vec2};
use opentern_core::Timeframe;
use opentern_engine::alerts::{Alert, Condition, Freq, Watch};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::time::Instant;

pub const TELEGRAM_TOKEN: &str = "telegram-bot-token";
pub const TELEGRAM_CHAT: &str = "telegram_chat_id";
/// Email: the SMTP password in the keychain; server, port, login and
/// recipient in settings.
pub const EMAIL_PASSWORD: &str = "email-smtp-password";
pub const EMAIL_HOST: &str = "email_smtp_host";
pub const EMAIL_PORT: &str = "email_smtp_port";
pub const EMAIL_USER: &str = "email_smtp_user";
pub const EMAIL_TO: &str = "email_to";

/// Where an alert's email goes.
#[derive(Clone, Debug)]
pub struct EmailTo {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub to: String,
}

/// Sends one email over SMTP (STARTTLS on 587 and others, TLS on 465).
pub fn send_email(e: &EmailTo, password: &str, subject: &str, body: &str) -> Result<(), String> {
    use lettre::message::header::ContentType;
    use lettre::transport::smtp::authentication::Credentials;
    use lettre::{Message, SmtpTransport, Transport};
    let from = e
        .user
        .parse()
        .map_err(|_| format!("\"{}\" isn't an email address.", e.user))?;
    let to =
        e.to.parse()
            .map_err(|_| format!("\"{}\" isn't an email address.", e.to))?;
    let msg = Message::builder()
        .from(from)
        .to(to)
        .subject(subject)
        .header(ContentType::TEXT_PLAIN)
        .body(body.to_string())
        .map_err(|err| err.to_string())?;
    let builder = if e.port == 465 {
        SmtpTransport::relay(&e.host)
    } else {
        SmtpTransport::starttls_relay(&e.host)
    }
    .map_err(|err| format!("Couldn't reach {}: {err}", e.host))?;
    let mailer = builder
        .port(e.port)
        .credentials(Credentials::new(e.user.clone(), password.to_string()))
        .timeout(Some(std::time::Duration::from_secs(20)))
        .build();
    mailer.send(&msg).map(|_| ()).map_err(|err| err.to_string())
}

/// One fired alert, as kept in the log.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Fired {
    pub alert_id: i64,
    pub name: String,
    pub symbol: String,
    pub time: i64,
    pub message: String,
}

/// What the delivery thread sends.
pub struct Delivery {
    pub title: String,
    pub body: String,
    pub notify: bool,
    pub sound: bool,
    pub webhook: String,
    pub telegram_chat: Option<String>,
    pub email: Option<EmailTo>,
}

pub struct Alerts {
    pub list: Vec<(Alert, Watch)>,
    pub log: Vec<Fired>,
    pub toasts: Vec<(Fired, Instant)>,
    /// Last values fed per alert (only new values are fed).
    fed: HashMap<i64, (f64, f64, i64)>,
    /// Script alerts already seen, per alert: (bar time, message).
    seen: HashMap<i64, HashSet<(i64, String)>>,
    tx: Option<Sender<Delivery>>,
    delivery: Option<std::thread::JoinHandle<()>>,
    /// Alerts whose symbol or study isn't on the chart.
    pub watcher: crate::watcher::Watcher,
}

pub fn now_ms() -> i64 {
    jiff::Timestamp::now().as_millisecond()
}

/// Starts the delivery thread (nothing is sent in headless runs).
fn deliverer() -> Option<(Sender<Delivery>, std::thread::JoinHandle<()>)> {
    if std::env::var_os("OPENTERN_HEADLESS").is_some() {
        return None;
    }
    let (tx, rx) = unbounded::<Delivery>();
    let handle = std::thread::Builder::new()
        .name("alert-delivery".into())
        .spawn(move || {
            for d in rx {
                deliver(&d);
            }
        })
        .ok()?;
    Some((tx, handle))
}

fn deliver(d: &Delivery) {
    if d.notify {
        let mut n = notify_rust::Notification::new();
        n.summary(&d.title).body(&d.body).appname("Opentern");
        if d.sound {
            n.sound_name("message-new-instant");
        }
        if let Err(e) = n.show() {
            crate::diag::crumb(format!("desktop notification failed: {e}"));
        }
    }
    if !d.webhook.trim().is_empty() {
        let json = serde_json::from_str::<serde_json::Value>(&d.body).is_ok();
        let r = ureq::post(d.webhook.trim())
            .header(
                "Content-Type",
                if json {
                    "application/json"
                } else {
                    "text/plain; charset=utf-8"
                },
            )
            .send(&d.body);
        // The URL may hold a secret: say which host failed, not the URL.
        if let Err(e) = r {
            let host = d.webhook.split('/').nth(2).unwrap_or("webhook");
            crate::diag::crumb(format!("webhook to {host} failed: {e}"));
        }
    }
    if let Some(chat) = &d.telegram_chat {
        match opentern_core::secrets::get(TELEGRAM_TOKEN).ok_or(()) {
            Ok(token) => {
                let url = format!("https://api.telegram.org/bot{token}/sendMessage");
                let text = format!("{}\n{}", d.title, d.body);
                if let Err(e) = ureq::post(&url).send_form([("chat_id", chat.as_str()), ("text", text.as_str())]) {
                    // Never log the URL (it holds the token).
                    let msg = e.to_string().replace(&token, "…");
                    crate::diag::crumb(format!("telegram failed: {msg}"));
                }
            }
            Err(_) => crate::diag::crumb("telegram: no bot token in the keychain"),
        }
    }
    if let Some(e) = &d.email {
        match opentern_core::secrets::get(EMAIL_PASSWORD) {
            Some(pw) => {
                if let Err(err) = send_email(e, &pw, &d.title, &d.body) {
                    crate::diag::crumb(format!("email to {} failed: {}", e.host, err.replace(&pw, "…")));
                }
            }
            None => crate::diag::crumb("email: no password in the keychain"),
        }
    }
}

impl Alerts {
    pub fn load(store: &opentern_core::Store, scripts: std::path::PathBuf) -> Alerts {
        let list = store
            .alerts()
            .into_iter()
            .filter_map(|(id, json)| {
                let mut a: Alert = serde_json::from_str(&json).ok()?;
                a.id = id;
                Some((a, Watch::default()))
            })
            .collect();
        let log = store
            .alert_log(500)
            .into_iter()
            .filter_map(|(_, _, json)| serde_json::from_str(&json).ok())
            .collect();
        let (tx, delivery) = deliverer().unzip();
        Alerts {
            list,
            log,
            toasts: Vec::new(),
            fed: HashMap::new(),
            seen: HashMap::new(),
            tx,
            delivery,
            watcher: crate::watcher::Watcher::new(scripts),
        }
    }

    /// Waits for messages still being sent (the background process,
    /// before it exits).
    pub fn flush(&mut self) {
        self.tx = None;
        if let Some(h) = self.delivery.take() {
            let _ = h.join();
        }
    }

    /// Saves an alert (new ones get their id) and keeps it in the list.
    pub fn save(&mut self, store: &mut opentern_core::Store, mut a: Alert) {
        if a.created == 0 {
            a.created = now_ms();
        }
        let json = match serde_json::to_string(&a) {
            Ok(j) => j,
            Err(e) => {
                crate::diag::crumb(format!("couldn't save an alert: {e}"));
                return;
            }
        };
        match store.save_alert(a.id, &json) {
            Ok(id) => {
                a.id = id;
                match self.list.iter_mut().find(|(x, _)| x.id == id) {
                    Some(slot) => *slot = (a, Watch::default()),
                    None => self.list.push((a, Watch::default())),
                }
                self.fed.remove(&id);
                self.seen.remove(&id);
            }
            Err(e) => crate::diag::crumb(format!("couldn't save an alert: {e}")),
        }
    }

    /// Feeds every active alert what it sees now (`probe`) and returns the
    /// ones that fire, with a script's message if it has one. Expired
    /// alerts are switched off and saved.
    pub fn evaluate(
        &mut self,
        store: &mut opentern_core::Store,
        mut probe: impl FnMut(&Alert, &mut crate::watcher::Watcher, i64) -> crate::watcher::Probe,
    ) -> Vec<(usize, String)> {
        use crate::watcher::Probe;
        let now = now_ms();
        let mut fire: Vec<(usize, String)> = Vec::new();
        let mut expired = Vec::new();
        for i in 0..self.list.len() {
            let a = &self.list[i].0;
            if !a.active {
                continue;
            }
            if a.expired(now) {
                expired.push(i);
                continue;
            }
            match probe(a, &mut self.watcher, now) {
                Probe::Nothing => {}
                Probe::Script(raised, forming) => {
                    let Condition::Script { title, .. } = &a.condition else {
                        continue;
                    };
                    let id = a.id;
                    let close_only = a.freq == Freq::OncePerBarClose;
                    let first = !self.seen.contains_key(&id);
                    let seen = self.seen.entry(id).or_default();
                    for (time, message, al_title) in raised {
                        let wanted = match title {
                            Some(t) => &al_title == t,
                            None => al_title.is_empty(),
                        };
                        if !wanted || (close_only && time >= forming) {
                            continue;
                        }
                        // Alerts from history don't fire when the alert is
                        // made or the app starts; only new ones do.
                        if seen.insert((time, message.clone())) && !first {
                            fire.push((i, message));
                        }
                    }
                }
                Probe::Values(v, l, b) => {
                    let id = a.id;
                    if self.fed.get(&id) == Some(&(v, l, b)) {
                        continue;
                    }
                    self.fed.insert(id, (v, l, b));
                    let (a, w) = &mut self.list[i];
                    if w.feed(a, v, l, now, b) {
                        fire.push((i, String::new()));
                    }
                }
            }
        }
        for i in expired {
            let mut a = self.list[i].0.clone();
            a.active = false;
            self.save(store, a);
        }
        fire
    }

    /// Fires alert `i`: counts it, logs it, and sends it where it asks.
    /// `price` fills `{{close}}`; a script's own message wins.
    pub fn fire(&mut self, store: &mut opentern_core::Store, i: usize, price: f64, script_message: String) -> Fired {
        let now = now_ms();
        let a = &mut self.list[i].0;
        a.fired += 1;
        a.last_fired = Some(now);
        if a.freq == Freq::Once {
            a.active = false;
        }
        let a = a.clone();
        let ticker = a.symbol.rsplit(':').next().unwrap_or(&a.symbol).to_string();
        let exchange = a.symbol.split_once(':').map(|x| x.0).unwrap_or("").to_string();
        let time = jiff::Timestamp::from_millisecond(now)
            .map(|t| t.to_string())
            .unwrap_or_default();
        let message = if !script_message.is_empty() {
            script_message
        } else if a.message.trim().is_empty() {
            format!("{} — {}", ticker, a.condition.summary())
        } else {
            opentern_engine::alerts::fill_message(&a.message, &ticker, &exchange, &a.tf, price, &time)
        };
        let name = if a.name.is_empty() {
            ticker.clone()
        } else {
            a.name.clone()
        };
        let f = Fired {
            alert_id: a.id,
            name: name.clone(),
            symbol: a.symbol.clone(),
            time: now,
            message: message.clone(),
        };
        if let Ok(j) = serde_json::to_string(&f)
            && let Err(e) = store.log_alert(a.id, now, &j)
        {
            crate::diag::crumb(format!("couldn't log an alert: {e}"));
        }
        self.log.insert(0, f.clone());
        self.log.truncate(500);
        crate::diag::crumb(format!("alert {} fired", a.id));
        if let Some(tx) = &self.tx {
            let chat = a
                .telegram
                .then(|| store.value(TELEGRAM_CHAT))
                .flatten()
                .filter(|c| !c.trim().is_empty());
            let email = a.email.then(|| email_to(store)).flatten();
            let _ = tx.send(Delivery {
                title: format!("Opentern: {name}"),
                body: message,
                notify: a.notify,
                sound: a.sound,
                webhook: a.webhook.clone(),
                telegram_chat: chat,
                email,
            });
        }
        self.save(store, a);
        f
    }
}

/// The email settings, if they're filled in.
pub(crate) fn email_to(store: &opentern_core::Store) -> Option<EmailTo> {
    let get = |k: &str| store.value(k).filter(|v| !v.trim().is_empty());
    Some(EmailTo {
        host: get(EMAIL_HOST)?,
        port: get(EMAIL_PORT).and_then(|p| p.trim().parse().ok()).unwrap_or(587),
        user: get(EMAIL_USER)?,
        to: get(EMAIL_TO)?,
    })
}

/// The open time of the `tf` bar containing `t` (fixed-length intervals;
/// months fall back to days).
fn bucket(tf: &str, t: i64) -> i64 {
    let ms = Timeframe::parse(tf).and_then(|t| t.millis()).unwrap_or(86_400_000);
    t.div_euclid(ms) * ms
}

impl App {
    /// Saves an alert (new ones get their id) and keeps it in the list.
    /// It also keeps what's needed to watch it with no chart: the
    /// symbol's details and, for plot and script alerts, the study.
    pub(crate) fn save_alert(&mut self, mut a: Alert) {
        self.fill_alert_context(&mut a);
        self.alerts.save(&mut self.store, a);
    }

    /// Fills an alert's symbol details and study from the charts showing
    /// them (unchanged when no chart has them).
    pub(crate) fn fill_alert_context(&self, a: &mut Alert) {
        let slots = self.slots.iter().flatten().map(|s| &s.chart);
        for c in std::iter::once(&self.chart).chain(slots) {
            if c.symbol.id == a.symbol {
                a.sym = Some(c.symbol.clone());
            }
            let uid = match &a.condition {
                Condition::Plot { uid, .. } | Condition::Script { uid, .. } => *uid,
                _ => continue,
            };
            if let Some(s) = c.studies.iter().find(|s| s.uid == uid) {
                a.study_spec = Some(opentern_engine::alerts::StudySpec {
                    id: s.meta().id.to_string(),
                    inputs: serde_json::to_value(&s.inst.inputs).unwrap_or_default(),
                });
            }
        }
        if a.sym.is_none() {
            a.sym = self
                .watchlists
                .iter()
                .flat_map(|w| w.symbols())
                .find(|s| s.id == a.symbol);
        }
    }

    /// Before the app closes: every alert keeps its symbol and study as
    /// they are now, for the background process.
    pub(crate) fn refresh_alert_context(&mut self) {
        for i in 0..self.alerts.list.len() {
            let mut a = self.alerts.list[i].0.clone();
            self.fill_alert_context(&mut a);
            if a != self.alerts.list[i].0 {
                self.alerts.save(&mut self.store, a);
            }
        }
    }

    pub(crate) fn delete_alert(&mut self, id: i64) {
        if let Err(e) = self.store.delete_alert(id) {
            crate::diag::crumb(format!("couldn't delete an alert: {e}"));
        }
        self.alerts.list.retain(|(a, _)| a.id != id);
    }

    /// Whether the active chart can answer for this alert itself: its
    /// symbol, and (plot and script alerts) its study.
    fn on_chart(&self, a: &Alert) -> bool {
        if a.symbol != self.chart.symbol.id {
            return false;
        }
        match &a.condition {
            Condition::Plot { uid, .. } | Condition::Script { uid, .. } => {
                self.chart.studies.iter().any(|s| s.uid == *uid)
            }
            _ => true,
        }
    }

    /// The chart's view of an alert: the value it watches, the level, and
    /// the bar.
    fn chart_probe(&self, a: &Alert, now: i64) -> crate::watcher::Probe {
        use crate::watcher::Probe;
        let Some(price) = self.chart.bars.last().map(|b| b.close) else {
            return Probe::Nothing;
        };
        let b = if a.tf == self.chart.tf.label() {
            self.chart
                .bars
                .last()
                .map(|b| b.time)
                .unwrap_or_else(|| bucket(&a.tf, now))
        } else {
            bucket(&a.tf, now)
        };
        match &a.condition {
            Condition::Price { value, .. } => Probe::Values(price, *value, b),
            Condition::Line { drawing, .. } => {
                // Follow the drawing if it moved.
                let cond = match drawing.and_then(|id| self.chart.drawings.iter().find(|d| d.id == id)) {
                    Some(d) => line_condition(d, opentern_engine::alerts::Op::Crossing).unwrap_or(a.condition.clone()),
                    None => a.condition.clone(),
                };
                match cond.line_price(now) {
                    Some(l) => Probe::Values(price, l, b),
                    None => Probe::Nothing,
                }
            }
            Condition::Plot { uid, plot, value, .. } => {
                let v = self
                    .chart
                    .studies
                    .iter()
                    .find(|s| s.uid == *uid)
                    .and_then(|s| s.inst.out.plots.get(*plot)?.last().copied());
                match v {
                    Some(v) => Probe::Values(v, value.unwrap_or(price), b),
                    None => Probe::Nothing,
                }
            }
            Condition::Script { uid, .. } => {
                let forming = self.chart.bars.last().map(|b| b.time).unwrap_or(i64::MAX);
                self.chart
                    .studies
                    .iter()
                    .find(|s| s.uid == *uid)
                    .and_then(|s| crate::watcher::script_alerts(&s.inst))
                    .map_or(Probe::Nothing, |l| Probe::Script(l, forming))
            }
        }
    }

    /// Every frame: checks alerts against new values and fires them. The
    /// chart answers for alerts on its symbol and studies; the watcher
    /// follows the rest (other symbols, studies no longer on the chart).
    pub(crate) fn poll_alerts(&mut self) {
        let off: Vec<Alert> = self
            .alerts
            .list
            .iter()
            .map(|(a, _)| a)
            .filter(|a| a.active && !self.on_chart(a))
            .cloned()
            .collect();
        let hub = self.hub.sender();
        self.alerts.watcher.set_scripts_dir(self.scripts_dir());
        self.alerts.watcher.sync(off.iter(), &|r| {
            let _ = hub.send(r);
        });
        let now = now_ms();
        let mut probes: HashMap<i64, crate::watcher::Probe> = self
            .alerts
            .list
            .iter()
            .filter(|(a, _)| a.active && self.on_chart(a))
            .map(|(a, _)| (a.id, self.chart_probe(a, now)))
            .collect();
        let fire = self
            .alerts
            .evaluate(&mut self.store, |a, w, now| match probes.remove(&a.id) {
                Some(p) => p,
                None => w.probe(a, now),
            });
        for (i, msg) in fire {
            self.fire_alert(i, msg);
        }
        self.alerts.toasts.retain(|(_, t)| t.elapsed().as_secs_f32() < 8.0);
        if !self.alerts.toasts.is_empty() {
            self.egui_ctx
                .request_repaint_after(std::time::Duration::from_millis(500));
        }
    }

    fn fire_alert(&mut self, i: usize, script_message: String) {
        let symbol = self.alerts.list[i].0.symbol.clone();
        let price = self
            .last_price(&symbol)
            .or_else(|| self.alerts.watcher.price(&symbol))
            .unwrap_or(f64::NAN);
        let f = self.alerts.fire(&mut self.store, i, price, script_message);
        self.alerts.toasts.push((f.clone(), Instant::now()));
        self.notify(crate::icons::ph::ALARM, &f.name, &f.message, None, false);
        self.egui_ctx.request_repaint();
    }

    /// Alerts that fired while the app was closed: one notification.
    pub(crate) fn alerts_fired_away(&mut self) {
        let key = crate::daemon::FIRED_AWAY;
        let n: u64 = self.store.value(key).and_then(|v| v.parse().ok()).unwrap_or(0);
        if n == 0 {
            return;
        }
        let _ = self.store.set_value(key, "0");
        let title = if n == 1 {
            "An alert fired while Opentern was closed".to_string()
        } else {
            format!("{n} alerts fired while Opentern was closed")
        };
        self.notify(crate::icons::ph::ALARM, &title, "", None, true);
    }

    /// The window is closing: alerts carry on in the background process
    /// if that's on and any is active.
    pub(crate) fn alerts_on_exit(&mut self) {
        self.refresh_alert_context();
        let Some(lock) = self.app_lock.take() else { return };
        drop(lock);
        let active = self.alerts.list.iter().any(|(a, _)| a.active);
        if active && crate::daemon::background_on(&self.store) && !crate::app::headless() {
            crate::daemon::spawn(self.store.dir(), self.demo);
        }
    }

    /// The email settings, if they're filled in.
    pub(crate) fn email_to(&self) -> Option<EmailTo> {
        email_to(&self.store)
    }

    /// Toasts for fired alerts, top right over the chart.
    pub(crate) fn alert_toasts(&mut self, ctx: &egui::Context) {
        if self.alerts.toasts.is_empty() {
            return;
        }
        let t = self.t();
        let screen = ctx.content_rect();
        let mut y = screen.top() + 56.0;
        let mut dismiss = None;
        for (k, (f, born)) in self.alerts.toasts.iter().enumerate().rev().take(4) {
            let age = born.elapsed().as_secs_f32();
            let alpha = ((8.0 - age) / 0.6).clamp(0.0, 1.0);
            let w = 300.0;
            egui::Area::new(egui::Id::new(("toast", k, f.time)))
                .fixed_pos(pos2(screen.right() - w - 60.0, y))
                .order(egui::Order::Foreground)
                .show(ctx, |ui| {
                    ui.set_opacity(alpha);
                    let r = egui::Frame::new()
                        .fill(t.elevated)
                        .stroke(Stroke::new(1.0, t.border))
                        .corner_radius(CornerRadius::same(10))
                        .inner_margin(egui::Margin::symmetric(14, 10))
                        .show(ui, |ui| {
                            ui.set_width(w - 28.0);
                            ui.horizontal(|ui| {
                                ui.label(
                                    egui::RichText::new(crate::icons::ph::ALARM)
                                        .font(theme::regular(15.0))
                                        .color(t.accent),
                                );
                                ui.label(egui::RichText::new(&f.name).font(theme::semibold(13.0)));
                            });
                            ui.label(egui::RichText::new(&f.message).color(t.text2).size(12.5));
                        });
                    if r.response.interact(Sense::click()).clicked() {
                        dismiss = Some(k);
                    }
                    y += r.response.rect.height() + 8.0;
                });
        }
        if let Some(k) = dismiss {
            self.alerts.toasts.remove(k);
        }
    }

    /// The Alerts widget: every alert, its state, and what it does.
    pub(crate) fn alerts_widget(&mut self, ui: &mut Ui, t: &Theme) {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Alerts").font(theme::semibold(14.0)));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if crate::widgets::tool_button(ui, t, crate::icons::ph::PLUS, "Create alert (Alt+A)", false, 26.0)
                    .clicked()
                {
                    self.open_alert_dialog(None);
                }
            });
        });
        ui.add_space(6.0);
        if self.alerts.list.is_empty() {
            crate::widgets::empty_state(ui, t, crate::icons::ph::ALARM, "No alerts yet");
            return;
        }
        let mut edit = None;
        let mut delete = None;
        let mut toggle = None;
        let now = now_ms();
        egui::ScrollArea::vertical().id_salt("alerts-list").show(ui, |ui| {
            for (a, _) in self.alerts.list.iter().rev() {
                let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 56.0), Sense::click());
                let p = ui.painter();
                // Pointer inside, not `hovered()`: the buttons would steal it.
                let inside = ui.rect_contains_pointer(r);
                if inside {
                    p.rect_filled(r, CornerRadius::same(6), t.hover_wash());
                }
                let (state, c) = if a.expired(now) {
                    ("Expired", t.text3)
                } else if a.active {
                    ("Active", t.pos)
                } else if a.fired > 0 && a.freq == Freq::Once {
                    ("Triggered", t.warn)
                } else {
                    ("Stopped", t.text3)
                };
                p.circle_filled(pos2(r.left() + 10.0, r.top() + 15.0), 4.0, c);
                let name = if a.name.is_empty() {
                    a.symbol.rsplit(':').next().unwrap_or(&a.symbol).to_string()
                } else {
                    a.name.clone()
                };
                p.text(
                    pos2(r.left() + 22.0, r.top() + 15.0),
                    Align2::LEFT_CENTER,
                    name,
                    theme::medium(13.0),
                    t.text,
                );
                p.text(
                    pos2(r.left() + 22.0, r.top() + 33.0),
                    Align2::LEFT_CENTER,
                    a.condition.summary(),
                    theme::regular(12.0),
                    t.text2,
                );
                let (line, c) = match self.alerts.watcher.error(a.id) {
                    Some(e) if a.active => (e.to_string(), t.warn),
                    _ => (format!("{state} · {} · fired {}×", a.freq.name(), a.fired), t.text3),
                };
                p.text(
                    pos2(r.left() + 22.0, r.top() + 48.0),
                    Align2::LEFT_CENTER,
                    line,
                    theme::regular(11.0),
                    c,
                );
                if inside {
                    // Pause/resume, edit and delete, on hover.
                    let mut x = r.right() - 4.0;
                    for (glyph, tip, what) in [
                        (crate::icons::ph::TRASH, "Delete", 2),
                        (crate::icons::ph::PENCIL_SIMPLE, "Edit", 1),
                        (
                            if a.active {
                                crate::icons::ph::PAUSE
                            } else {
                                crate::icons::ph::PLAY
                            },
                            if a.active { "Stop" } else { "Restart" },
                            0,
                        ),
                    ] {
                        let br = egui::Rect::from_min_size(pos2(x - 24.0, r.top() + 4.0), vec2(24.0, 24.0));
                        x -= 26.0;
                        let b = ui.interact(br, ui.id().with((a.id, what)), Sense::click());
                        let col = if b.hovered() { t.text } else { t.text2 };
                        ui.painter()
                            .text(br.center(), Align2::CENTER_CENTER, glyph, theme::regular(14.0), col);
                        if b.on_hover_text(tip).clicked() {
                            match what {
                                0 => toggle = Some(a.id),
                                1 => edit = Some(a.id),
                                _ => delete = Some(a.id),
                            }
                        }
                    }
                }
                if resp.double_clicked() {
                    edit = Some(a.id);
                }
                ui.painter()
                    .hline(r.x_range(), r.bottom(), Stroke::new(1.0, t.border.gamma_multiply(0.5)));
            }
        });
        if let Some(id) = toggle
            && let Some((a, _)) = self.alerts.list.iter().find(|(a, _)| a.id == id)
        {
            let mut a = a.clone();
            a.active = !a.active;
            if a.active && a.expired(now) {
                a.expires = None;
            }
            self.save_alert(a);
        }
        if let Some(id) = edit
            && let Some((a, _)) = self.alerts.list.iter().find(|(a, _)| a.id == id)
        {
            self.open_alert_dialog(Some(a.clone()));
        }
        if let Some(id) = delete {
            self.delete_alert(id);
        }
    }

    /// The Alerts log tab.
    pub(crate) fn alert_log_tab(&mut self, ui: &mut Ui, t: &Theme) {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(format!("{} alerts fired", self.alerts.log.len())).color(t.text2));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if !self.alerts.log.is_empty() && small_button(ui, t, "Clear log", false).clicked() {
                    if let Err(e) = self.store.clear_alert_log() {
                        crate::diag::crumb(format!("couldn't clear the alert log: {e}"));
                    }
                    self.alerts.log.clear();
                }
            });
        });
        ui.add_space(6.0);
        if self.alerts.log.is_empty() {
            crate::widgets::empty_state(
                ui,
                t,
                crate::icons::ph::CLOCK_COUNTER_CLOCKWISE,
                "Nothing has fired yet",
            );
            return;
        }
        let cols = [("Time", 150.0), ("Alert", 160.0), ("Symbol", 120.0), ("Message", 480.0)];
        let w: f32 = cols.iter().map(|c| c.1).sum();
        let (hr, _) = ui.allocate_exact_size(vec2(w, 24.0), Sense::hover());
        let mut x = hr.left();
        for (name, cw) in cols {
            ui.painter().text(
                pos2(x + 8.0, hr.center().y),
                Align2::LEFT_CENTER,
                name,
                theme::medium(12.0),
                t.text3,
            );
            x += cw;
        }
        ui.painter()
            .hline(hr.x_range(), hr.bottom(), Stroke::new(1.0, t.border));
        let tz = self.chart.tz.clone();
        let n = self.alerts.log.len();
        egui::ScrollArea::vertical()
            .id_salt("alert-log")
            .auto_shrink([false; 2])
            .show_rows(ui, 24.0, n, |ui, range| {
                for k in range {
                    let f = &self.alerts.log[k];
                    let (r, resp) = ui.allocate_exact_size(vec2(w, 24.0), Sense::hover());
                    if resp.hovered() {
                        ui.painter().rect_filled(r, CornerRadius::same(4), t.hover_wash());
                    }
                    let time = jiff::Timestamp::from_millisecond(f.time)
                        .map(|ts| ts.to_zoned(tz.clone()).strftime("%Y-%m-%d %H:%M:%S").to_string())
                        .unwrap_or_default();
                    let sym = f.symbol.rsplit(':').next().unwrap_or(&f.symbol).to_string();
                    let cells: [(String, Color32); 4] = [
                        (time, t.text2),
                        (f.name.clone(), t.text),
                        (sym, t.text),
                        (f.message.clone(), t.text2),
                    ];
                    let mut x = r.left();
                    for ((_, cw), (text, c)) in cols.iter().zip(cells) {
                        let cell = egui::Rect::from_min_size(pos2(x, r.top()), vec2(cw - 8.0, 24.0));
                        ui.painter_at(cell).text(
                            pos2(x + 8.0, r.center().y),
                            Align2::LEFT_CENTER,
                            text,
                            theme::regular(12.5),
                            c,
                        );
                        x += cw;
                    }
                    ui.painter()
                        .hline(r.x_range(), r.bottom(), Stroke::new(1.0, t.border.gamma_multiply(0.5)));
                }
            });
    }
}

/// A line drawing as an alert condition (`None` for other tools).
pub fn line_condition(d: &crate::chart::draw::Drawing, op: opentern_engine::alerts::Op) -> Option<Condition> {
    let (a, b) = (d.pts.first()?, d.pts.get(1).or(d.pts.first())?);
    let (extend, t1, p1, t2, p2) = match d.tool {
        Tool::TrendLine => (false, a.t, a.p, b.t, b.p),
        Tool::Ray | Tool::ExtendedLine => (true, a.t, a.p, b.t, b.p),
        Tool::HorizontalLine | Tool::HorizontalRay => (true, 0, a.p, 1, a.p),
        _ => return None,
    };
    Some(Condition::Line {
        op,
        t1,
        p1,
        t2,
        p2,
        extend,
        drawing: Some(d.id),
    })
}

#[cfg(test)]
mod tests {

    #[test]
    fn email_errors_are_plain() {
        let to = EmailTo {
            host: "smtp.example.invalid".into(),
            port: 587,
            user: "not an address".into(),
            to: "you@example.com".into(),
        };
        let e = send_email(&to, "pw", "s", "b").unwrap_err();
        assert!(e.contains("isn't an email address"), "{e}");
    }

    use super::*;

    #[test]
    fn buckets() {
        assert_eq!(bucket("1h", 3_600_000 * 5 + 10), 3_600_000 * 5);
        assert_eq!(bucket("nonsense", 86_400_000 + 1), 86_400_000);
    }
}

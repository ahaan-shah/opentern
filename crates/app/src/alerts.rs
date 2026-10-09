//! Alerts in the app: evaluated as prices arrive (whether or not the
//! symbol is on the chart, for price alerts), fired into the Alerts log, a
//! toast, and the channels the alert asks for. Desktop notifications,
//! webhooks and Telegram are sent from a background thread, so a slow
//! network never stalls a frame. The Telegram bot token lives in the OS
//! keychain.

use crate::app::App;
use crate::chart::draw::Tool;
use crate::shell::small_button;
use crate::theme::{self, Theme};
use crossbeam_channel::{Sender, unbounded};
use egui::{Align, Align2, Color32, CornerRadius, Layout, Sense, Stroke, Ui, pos2, vec2};
use opentern_core::Timeframe;
use opentern_engine::alerts::{Alert, Condition, Freq, Watch};
use opentern_pine::PineStudy;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::time::Instant;

pub const KEYCHAIN_SERVICE: &str = "opentern";
pub const TELEGRAM_TOKEN: &str = "telegram-bot-token";
pub const TELEGRAM_CHAT: &str = "telegram_chat_id";

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
}

pub fn now_ms() -> i64 {
    jiff::Timestamp::now().as_millisecond()
}

/// Starts the delivery thread (nothing is sent in headless runs).
fn deliverer() -> Option<Sender<Delivery>> {
    if std::env::var_os("OPENTERN_HEADLESS").is_some() {
        return None;
    }
    let (tx, rx) = unbounded::<Delivery>();
    std::thread::Builder::new()
        .name("alert-delivery".into())
        .spawn(move || {
            for d in rx {
                deliver(&d);
            }
        })
        .ok()?;
    Some(tx)
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
        match keyring::Entry::new(KEYCHAIN_SERVICE, TELEGRAM_TOKEN).and_then(|e| e.get_password()) {
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
}

impl Alerts {
    pub fn load(store: &opentern_core::Store) -> Alerts {
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
        Alerts {
            list,
            log,
            toasts: Vec::new(),
            fed: HashMap::new(),
            seen: HashMap::new(),
            tx: deliverer(),
        }
    }
}

/// The open time of the `tf` bar containing `t` (fixed-length intervals;
/// months fall back to days).
fn bucket(tf: &str, t: i64) -> i64 {
    let ms = Timeframe::parse(tf).and_then(|t| t.millis()).unwrap_or(86_400_000);
    t.div_euclid(ms) * ms
}

impl App {
    /// Saves an alert (new ones get their id) and keeps it in the list.
    pub(crate) fn save_alert(&mut self, mut a: Alert) {
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
        match self.store.save_alert(a.id, &json) {
            Ok(id) => {
                a.id = id;
                match self.alerts.list.iter_mut().find(|(x, _)| x.id == id) {
                    Some(slot) => *slot = (a, Watch::default()),
                    None => self.alerts.list.push((a, Watch::default())),
                }
                self.alerts.fed.remove(&id);
                self.alerts.seen.remove(&id);
            }
            Err(e) => crate::diag::crumb(format!("couldn't save an alert: {e}")),
        }
    }

    pub(crate) fn delete_alert(&mut self, id: i64) {
        if let Err(e) = self.store.delete_alert(id) {
            crate::diag::crumb(format!("couldn't delete an alert: {e}"));
        }
        self.alerts.list.retain(|(a, _)| a.id != id);
    }

    /// The value an alert watches and the level it's compared with, now.
    fn alert_inputs(&self, a: &Alert, now: i64) -> Option<(f64, f64, i64)> {
        let chart_sym = a.symbol == self.chart.symbol.id;
        let price = self.last_price(&a.symbol)?;
        let b = if chart_sym && a.tf == self.chart.tf.label() {
            self.chart
                .bars
                .last()
                .map(|b| b.time)
                .unwrap_or_else(|| bucket(&a.tf, now))
        } else {
            bucket(&a.tf, now)
        };
        match &a.condition {
            Condition::Price { value, .. } => Some((price, *value, b)),
            Condition::Line { drawing, .. } => {
                // Follow the drawing if it moved.
                let cond = match drawing.and_then(|id| self.chart.drawings.iter().find(|d| d.id == id)) {
                    Some(d) if chart_sym => {
                        line_condition(d, opentern_engine::alerts::Op::Crossing).unwrap_or(a.condition.clone())
                    }
                    _ => a.condition.clone(),
                };
                Some((price, cond.line_price(now)?, b))
            }
            Condition::Plot { uid, plot, value, .. } => {
                if !chart_sym {
                    return None;
                }
                let s = self.chart.studies.iter().find(|s| s.uid == *uid)?;
                let v = *s.inst.out.plots.get(*plot)?.last()?;
                Some((v, value.unwrap_or(price), b))
            }
            Condition::Script { .. } => None,
        }
    }

    /// Every frame: checks alerts against new values and fires them.
    pub(crate) fn poll_alerts(&mut self) {
        let now = now_ms();
        let mut fire: Vec<(usize, String)> = Vec::new();
        for i in 0..self.alerts.list.len() {
            let a = &self.alerts.list[i].0;
            if !a.active {
                continue;
            }
            if a.expired(now) {
                let mut a = a.clone();
                a.active = false;
                self.save_alert(a);
                continue;
            }
            if let Condition::Script { uid, title, .. } = &a.condition {
                let id = a.id;
                let close_only = a.freq == Freq::OncePerBarClose;
                let title = title.clone();
                let Some(st) = self
                    .chart
                    .studies
                    .iter()
                    .find(|s| s.uid == *uid)
                    .and_then(|s| s.inst.latest_state())
                    .and_then(|x| x.downcast_ref::<PineStudy>())
                else {
                    continue;
                };
                let forming = self.chart.bars.last().map(|b| b.time).unwrap_or(i64::MAX);
                let first = !self.alerts.seen.contains_key(&id);
                let seen = self.alerts.seen.entry(id).or_default();
                for al in st.alerts() {
                    let wanted = match &title {
                        Some(t) => &al.title == t,
                        None => al.title.is_empty(),
                    };
                    if !wanted || (close_only && al.time >= forming) {
                        continue;
                    }
                    // Alerts from history don't fire when the alert is made
                    // or the app starts; only new ones do.
                    if seen.insert((al.time, al.message.clone())) && !first {
                        fire.push((i, al.message.clone()));
                    }
                }
                continue;
            }
            let Some((v, l, b)) = self.alert_inputs(a, now) else {
                continue;
            };
            let id = a.id;
            if self.alerts.fed.get(&id) == Some(&(v, l, b)) {
                continue;
            }
            self.alerts.fed.insert(id, (v, l, b));
            let (a, w) = &mut self.alerts.list[i];
            if w.feed(a, v, l, now, b) {
                fire.push((i, String::new()));
            }
        }
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
        let now = now_ms();
        let price = self.last_price(&self.alerts.list[i].0.symbol).unwrap_or(f64::NAN);
        let a = &mut self.alerts.list[i].0;
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
            && let Err(e) = self.store.log_alert(a.id, now, &j)
        {
            crate::diag::crumb(format!("couldn't log an alert: {e}"));
        }
        self.alerts.log.insert(0, f.clone());
        self.alerts.log.truncate(500);
        self.alerts.toasts.push((f, Instant::now()));
        crate::diag::crumb(format!("alert {} fired", a.id));
        if let Some(tx) = &self.alerts.tx {
            let chat = a
                .telegram
                .then(|| self.store.value(TELEGRAM_CHAT))
                .flatten()
                .filter(|c| !c.trim().is_empty());
            let _ = tx.send(Delivery {
                title: format!("Opentern: {name}"),
                body: message,
                notify: a.notify,
                sound: a.sound,
                webhook: a.webhook.clone(),
                telegram_chat: chat,
            });
        }
        self.save_alert(a);
        self.egui_ctx.request_repaint();
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
            ui.label(
                egui::RichText::new("No alerts yet. Press Alt+A to make one for the chart's symbol.").color(t.text3),
            );
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
                if resp.hovered() {
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
                p.text(
                    pos2(r.left() + 22.0, r.top() + 48.0),
                    Align2::LEFT_CENTER,
                    format!("{state} · {} · fired {}×", a.freq.name(), a.fired),
                    theme::regular(11.0),
                    t.text3,
                );
                if resp.hovered() {
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
            ui.label(egui::RichText::new("Alerts that fire show up here.").color(t.text3));
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
    use super::*;

    #[test]
    fn buckets() {
        assert_eq!(bucket("1h", 3_600_000 * 5 + 10), 3_600_000 * 5);
        assert_eq!(bucket("nonsense", 86_400_000 + 1), 86_400_000);
    }
}

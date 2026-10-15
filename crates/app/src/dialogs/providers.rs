//! Settings → Data providers: keys for the sources that need an account.
//! Secrets go to the OS keychain; the rest (account ids, URLs, flags) to
//! the workspace settings. Each section says where to get its key.

use super::{Done, form_row, frame, section};
use crate::app::App;
use crate::theme::Theme;
use egui::Ui;
use opentern_core::secrets;
use opentern_data::Credentials;
use opentern_data::creds::keys;

pub const ALPACA_SIP: &str = "alpaca_sip";
pub const OANDA_ACCOUNT: &str = "oanda_account";
pub const OANDA_LIVE: &str = "oanda_live";
pub const IBKR_URL: &str = "ibkr_url";

pub struct Providers {
    alpaca_key: String,
    alpaca_secret: String,
    alpaca_sip: bool,
    oanda_token: String,
    oanda_account: String,
    oanda_live: bool,
    upstox_token: String,
    ibkr_on: bool,
    ibkr_url: String,
    /// Which secrets are already saved (fields stay empty: paste to replace).
    have: [bool; 4],
    error: String,
    /// Saved from the Settings page (shows "Saved.").
    saved: bool,
}

/// The credentials in the keychain and settings (call off the UI thread:
/// a locked keychain may ask to be unlocked).
pub fn load_credentials(settings: &[(String, Option<String>)]) -> Credentials {
    let get = |k: &str| settings.iter().find(|(n, _)| n == k).and_then(|(_, v)| v.clone());
    let alpaca = match (secrets::get(keys::ALPACA_KEY), secrets::get(keys::ALPACA_SECRET)) {
        (Some(k), Some(s)) => Some((k, s, get(ALPACA_SIP).as_deref() == Some("1"))),
        _ => None,
    };
    let oanda = match (secrets::get(keys::OANDA_TOKEN), get(OANDA_ACCOUNT)) {
        (Some(t), Some(a)) if !a.is_empty() => Some((t, a, get(OANDA_LIVE).as_deref() == Some("1"))),
        _ => None,
    };
    Credentials {
        alpaca,
        oanda,
        upstox: secrets::get(keys::UPSTOX_TOKEN),
        ibkr: get(IBKR_URL).filter(|u| !u.is_empty()),
    }
}

impl App {
    /// The non-secret provider settings, to read with the keychain.
    pub(crate) fn provider_settings(&self) -> Vec<(String, Option<String>)> {
        [ALPACA_SIP, OANDA_ACCOUNT, OANDA_LIVE, IBKR_URL]
            .iter()
            .map(|k| (k.to_string(), self.store.value(k)))
            .collect()
    }

    /// Reads the keys (on a background thread) and hands them to the hub.
    pub(crate) fn send_credentials(&self) {
        let settings = self.provider_settings();
        let tx = self.hub.sender();
        let _ = std::thread::Builder::new().name("credentials".into()).spawn(move || {
            if std::env::var_os("OPENTERN_HEADLESS").is_some() {
                return;
            }
            let c = load_credentials(&settings);
            let _ = tx.send(opentern_data::Request::Credentials(c));
        });
    }

    pub(crate) fn open_providers(&mut self) {
        self.modal = Some(super::Modal::Providers(Box::new(self.providers_form())));
    }

    /// The providers form, filled from the keychain and settings.
    pub(crate) fn providers_form(&self) -> Providers {
        let v = |k: &str| self.store.value(k).unwrap_or_default();
        let have = [
            secrets::get(keys::ALPACA_KEY).is_some(),
            secrets::get(keys::ALPACA_SECRET).is_some(),
            secrets::get(keys::OANDA_TOKEN).is_some(),
            secrets::get(keys::UPSTOX_TOKEN).is_some(),
        ];
        Providers {
            alpaca_key: String::new(),
            alpaca_secret: String::new(),
            alpaca_sip: v(ALPACA_SIP) == "1",
            oanda_token: String::new(),
            oanda_account: v(OANDA_ACCOUNT),
            oanda_live: v(OANDA_LIVE) == "1",
            upstox_token: String::new(),
            ibkr_on: !v(IBKR_URL).is_empty(),
            ibkr_url: if v(IBKR_URL).is_empty() {
                "https://localhost:5000".into()
            } else {
                v(IBKR_URL)
            },
            have,
            error: String::new(),
            saved: false,
        }
    }
}

fn secret_field(ui: &mut Ui, value: &mut String, saved: bool, hint: &str) {
    let hint = if saved {
        "Saved in your keychain (paste to replace)"
    } else {
        hint
    };
    ui.add(
        crate::widgets::field(value)
            .password(true)
            .hint_text(hint)
            .desired_width(260.0),
    );
}

fn note(ui: &mut Ui, t: &Theme, text: &str) {
    ui.label(egui::RichText::new(text).color(t.text3).size(11.5));
}

impl Providers {
    pub fn show(&mut self, ui: &mut Ui, app: &mut App, t: &Theme) -> Done {
        let mut done = Done::Keep;
        let max_h = (ui.ctx().content_rect().height() - 190.0).max(200.0);
        let close = frame(ui, t, "Data providers", 480.0, |ui| {
            note(
                ui,
                t,
                "Crypto (Binance, Bybit, OKX, Coinbase, Kraken) and Yahoo need nothing. Add a key to use a source \
                 that needs an account. Keys stay in your system keychain.",
            );
            egui::ScrollArea::vertical()
                .max_height(max_h)
                .show(ui, |ui| self.fields(ui, t));
            if !self.error.is_empty() {
                ui.add_space(4.0);
                ui.label(egui::RichText::new(&self.error).color(t.neg));
            }
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if crate::widgets::primary(ui, "Save").clicked() {
                        match self.save(app) {
                            Ok(()) => done = Done::Close,
                            Err(e) => self.error = e,
                        }
                    }
                    if crate::widgets::secondary(ui, "Cancel").clicked() {
                        done = Done::Close;
                    }
                });
            });
        });
        if close {
            done = Done::Close;
        }
        done
    }

    /// The form's sections (shared by the dialog and the Settings page).
    fn fields(&mut self, ui: &mut Ui, t: &Theme) {
        section(ui, t, "Alpaca · US stocks");
        note(
            ui,
            t,
            "Free real-time prices (IEX). Keys: app.alpaca.markets → API keys.",
        );
        form_row(ui, t, "Key ID", |ui| {
            secret_field(ui, &mut self.alpaca_key, self.have[0], "PK…")
        });
        form_row(ui, t, "Secret key", |ui| {
            secret_field(ui, &mut self.alpaca_secret, self.have[1], "")
        });
        form_row(ui, t, "Feed", |ui| {
            crate::widgets::checkbox(ui, &mut self.alpaca_sip, "My plan includes SIP (all US exchanges)");
        });
        section(ui, t, "OANDA · forex and CFDs");
        note(
            ui,
            t,
            "Token: OANDA → My Services → Manage API Access. Use symbols like OANDA:EURUSD.",
        );
        form_row(ui, t, "API token", |ui| {
            secret_field(ui, &mut self.oanda_token, self.have[2], "")
        });
        form_row(ui, t, "Account ID", |ui| {
            ui.add(
                crate::widgets::field(&mut self.oanda_account)
                    .hint_text("101-…")
                    .desired_width(260.0),
            );
        });
        form_row(ui, t, "Account", |ui| {
            crate::widgets::radio(ui, &mut self.oanda_live, false, "Practice");
            crate::widgets::radio(ui, &mut self.oanda_live, true, "Live");
        });
        section(ui, t, "Upstox · NSE and BSE");
        note(
            ui,
            t,
            "Real-time Indian prices with an Upstox account. Make an app at upstox.com/developer and paste \
                 its access token (it lasts until the next morning).",
        );
        form_row(ui, t, "Access token", |ui| {
            secret_field(ui, &mut self.upstox_token, self.have[3], "")
        });
        section(ui, t, "Interactive Brokers");
        note(
            ui,
            t,
            "Run IBKR's Client Portal gateway and log in; then use symbols like IBKR:AAPL. Needs IBKR \
                 market-data subscriptions.",
        );
        form_row(ui, t, "Use the gateway", |ui| {
            crate::widgets::checkbox(ui, &mut self.ibkr_on, "");
        });
        if self.ibkr_on {
            form_row(ui, t, "Gateway URL", |ui| {
                ui.add(crate::widgets::field(&mut self.ibkr_url).desired_width(260.0));
            });
        }
    }

    /// The Settings page's version: no frame, Save in line.
    pub fn page(&mut self, ui: &mut Ui, app: &mut App, t: &Theme) {
        note(
            ui,
            t,
            "Crypto (Binance, Bybit, OKX, Coinbase, Kraken) and Yahoo need nothing. Add a key to use a source \
             that needs an account. Keys stay in your system keychain.",
        );
        let before = (
            self.alpaca_key.clone(),
            self.alpaca_secret.clone(),
            self.oanda_token.clone(),
            self.upstox_token.clone(),
        );
        self.fields(ui, t);
        if before
            != (
                self.alpaca_key.clone(),
                self.alpaca_secret.clone(),
                self.oanda_token.clone(),
                self.upstox_token.clone(),
            )
        {
            self.saved = false;
        }
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            if crate::widgets::primary(ui, "Save").clicked() {
                match self.save(app) {
                    Ok(()) => {
                        self.saved = true;
                        self.error.clear();
                    }
                    Err(e) => self.error = e,
                }
            }
            if !self.error.is_empty() {
                ui.label(egui::RichText::new(&self.error).color(t.neg));
            } else if self.saved {
                ui.label(egui::RichText::new("Saved.").color(t.text3));
            }
        });
    }

    fn save(&mut self, app: &mut App) -> Result<(), String> {
        let put =
            |k: &str, v: &str| secrets::set(k, v.trim()).map_err(|e| format!("Couldn't save to your keychain: {e}"));
        if !self.alpaca_key.trim().is_empty() {
            put(keys::ALPACA_KEY, &self.alpaca_key)?;
        }
        if !self.alpaca_secret.trim().is_empty() {
            put(keys::ALPACA_SECRET, &self.alpaca_secret)?;
        }
        if !self.oanda_token.trim().is_empty() {
            put(keys::OANDA_TOKEN, &self.oanda_token)?;
        }
        if !self.upstox_token.trim().is_empty() {
            put(keys::UPSTOX_TOKEN, &self.upstox_token)?;
        }
        let url = if self.ibkr_on { self.ibkr_url.trim() } else { "" };
        if self.ibkr_on && !url.starts_with("http") {
            return Err("The gateway URL should look like https://localhost:5000".into());
        }
        let flag = |b: bool| if b { "1" } else { "0" };
        for (k, v) in [
            (ALPACA_SIP, flag(self.alpaca_sip)),
            (OANDA_ACCOUNT, self.oanda_account.trim()),
            (OANDA_LIVE, flag(self.oanda_live)),
            (IBKR_URL, url),
        ] {
            app.store.set_value(k, v).map_err(|e| format!("Couldn't save: {e}"))?;
        }
        // New keys take effect at once: hand them over and reopen the chart.
        let c = load_credentials(&app.provider_settings());
        app.hub.send(opentern_data::Request::Credentials(c));
        app.open_chart_data();
        Ok(())
    }
}

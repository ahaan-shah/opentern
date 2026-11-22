//! A paper account's settings: name, starting capital, currency,
//! commission, slippage and leverage. The same dialog makes a new account
//! and edits the one in use.

use super::{Done, form_row, frame};
use crate::app::App;
use crate::theme::Theme;
use egui::Ui;

pub struct PaperAccountForm {
    /// Making a new account (else editing the one in use).
    pub new: bool,
    pub name: String,
    pub capital: f64,
    /// Percent of each fill's value.
    pub commission: f64,
    /// Percent of the price, against market, stop and trailing fills.
    pub slippage: f64,
    /// 1 = cash only.
    pub leverage: f64,
    /// "" = each symbol's own money.
    pub currency: String,
    /// The account has traded: its money stays in its currency.
    pub currency_locked: bool,
}

/// The currencies an account can be kept in ("" = no conversion).
const CURRENCIES: [&str; 11] = ["", "USD", "INR", "EUR", "GBP", "JPY", "AUD", "CAD", "CHF", "SGD", "HKD"];

fn currency_name(c: &str) -> &str {
    if c.is_empty() { "Each symbol's own" } else { c }
}

impl PaperAccountForm {
    pub fn show(&mut self, ui: &mut Ui, app: &mut App, t: &Theme) -> Done {
        let lang = app.language;
        let mut done = Done::Keep;
        let title = if self.new {
            lang.text("New paper account")
        } else {
            lang.text("Account settings")
        };
        if frame(ui, t, title, 400.0, |ui| {
            let mut enter = false;
            form_row(ui, t, lang.text("Name"), |ui| {
                let r = ui.add(crate::widgets::field(&mut self.name).desired_width(ui.available_width()));
                enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            });
            form_row(ui, t, lang.text("Starting capital"), |ui| {
                let r = ui.add(
                    egui::DragValue::new(&mut self.capital)
                        .speed(100.0)
                        .range(1.0..=1e15)
                        .max_decimals(2),
                );
                if !self.new {
                    r.on_hover_text(lang.text("Profit and loss made so far is kept"));
                }
            });
            form_row(ui, t, lang.text("Currency"), |ui| {
                let locked = self.currency_locked;
                ui.add_enabled_ui(!locked, |ui| {
                    crate::widgets::combo("paper-ccy")
                        .selected_text(lang.text(currency_name(&self.currency)))
                        .width(170.0)
                        .show_ui(ui, |ui| {
                            for c in CURRENCIES {
                                crate::widgets::option_value(
                                    ui,
                                    &mut self.currency,
                                    c.to_string(),
                                    lang.text(currency_name(c)),
                                );
                            }
                        });
                })
                .response
                .on_disabled_hover_text(lang.text("Reset the account to change its currency"))
                .on_hover_text(lang.text("Profit and loss in other currencies is converted at live rates"));
            });
            form_row(ui, t, lang.text("Commission"), |ui| {
                ui.add(
                    egui::DragValue::new(&mut self.commission)
                        .speed(0.01)
                        .range(0.0..=100.0)
                        .max_decimals(4)
                        .suffix(" %"),
                )
                .on_hover_text(lang.text("Of each fill's value"));
            });
            form_row(ui, t, lang.text("Slippage"), |ui| {
                ui.add(
                    egui::DragValue::new(&mut self.slippage)
                        .speed(0.01)
                        .range(0.0..=10.0)
                        .max_decimals(4)
                        .suffix(" %"),
                )
                .on_hover_text(lang.text("Against market, stop and trailing fills; limits fill at their price"));
            });
            form_row(ui, t, lang.text("Leverage"), |ui| {
                ui.add(
                    egui::DragValue::new(&mut self.leverage)
                        .speed(0.1)
                        .range(1.0..=100.0)
                        .max_decimals(2)
                        .suffix("×"),
                )
                .on_hover_text(lang.text("1× is cash only. Below half the margin in use, every position is closed"));
            });
            let name = self.name.trim();
            let taken = app.paper_name_taken(name, self.new);
            if taken {
                ui.add_space(4.0);
                ui.label(
                    egui::RichText::new(lang.text("There's already an account with this name."))
                        .color(t.warn)
                        .size(12.0),
                );
            }
            let ok = !name.is_empty() && !taken;
            ui.add_space(14.0);
            ui.horizontal(|ui| {
                ui.add_space((ui.available_width() - 160.0).max(0.0));
                if crate::widgets::secondary(ui, lang.text("Cancel")).clicked() {
                    done = Done::Close;
                }
                let label = if self.new {
                    lang.text("Create")
                } else {
                    lang.text("Save")
                };
                let go = crate::widgets::button(ui, crate::widgets::Kind::Primary, ok, None, label).clicked();
                if ok && (go || enter) {
                    app.apply_paper_account(self);
                    done = Done::Close;
                }
            });
        }) {
            done = Done::Close;
        }
        done
    }
}

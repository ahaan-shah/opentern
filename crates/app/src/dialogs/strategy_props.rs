//! The Strategy Tester's Properties: capital, order size, pyramiding,
//! commission, slippage, margin and fills, overriding what the script's
//! `strategy(...)` says. Overrides are saved per script file (settings key
//! `strategy_props:<file>`), so they follow the script onto any chart, and
//! "Reset" goes back to the script's own values.

use super::{Done, form_row, frame, section};
use crate::app::App;
use crate::theme::Theme;
use egui::Ui;
use opentern_engine::{Commission, Props, QtyType};

pub struct StrategyProps {
    file: String,
    name: String,
    /// What the script declares.
    base: Props,
    p: Props,
}

const KEY: &str = "strategy_props:";

impl App {
    /// Saved overrides of a script's `strategy(...)` properties.
    pub(crate) fn strategy_overrides(&self, file: &str) -> serde_json::Value {
        self.store
            .value(&format!("{KEY}{file}"))
            .and_then(|j| serde_json::from_str(&j).ok())
            .unwrap_or_else(|| serde_json::json!({}))
    }

    /// Saves overrides and reruns every copy of the script.
    pub(crate) fn set_strategy_overrides(&mut self, file: &str, over: serde_json::Value) {
        let empty = over.as_object().is_none_or(|o| o.is_empty());
        let key = format!("{KEY}{file}");
        let _ = if empty {
            self.store.set_value(&key, "")
        } else {
            self.store.set_value(&key, &over.to_string())
        };
        let file = file.to_string();
        self.each_chart(|a| {
            for i in 0..a.scripts.len() {
                if a.scripts[i].file == file && a.scripts[i].lang.in_process() {
                    a.compile_pine(i);
                }
            }
            a.strategies.key = (0, 0, 0);
        });
    }

    /// Opens Properties for the strategy on the chart with study `uid`.
    pub(crate) fn open_strategy_props(&mut self, uid: u64) {
        let Some(run) = self.scripts.iter().find(|r| r.uid == Some(uid)) else {
            return;
        };
        let mintick = self.chart.symbol.tick_size;
        let base = Props::from_json(&run.props, mintick);
        let over = self.strategy_overrides(&run.file);
        let p = Props::from_json(&opentern_engine::merge_json(&run.props, &over), mintick);
        let name = self
            .chart
            .studies
            .iter()
            .find(|s| s.uid == uid)
            .map(|s| s.meta().short.to_string())
            .unwrap_or_else(|| run.file.clone());
        self.modal = Some(super::Modal::StrategyProps(Box::new(StrategyProps {
            file: run.file.clone(),
            name,
            base,
            p,
        })));
    }
}

fn number(ui: &mut Ui, v: &mut f64, speed: f64, min: f64, max: f64, suffix: &str) {
    ui.add(
        egui::DragValue::new(v)
            .speed(speed)
            .range(min..=max)
            .max_decimals(6)
            .suffix(suffix),
    );
}

impl StrategyProps {
    pub fn show(&mut self, ui: &mut Ui, app: &mut App, t: &Theme) -> Done {
        let mut done = Done::Keep;
        let title = format!("{} — properties", self.name);
        if frame(ui, t, &title, 440.0, |ui| {
            let p = &mut self.p;
            section(ui, t, "Capital and size");
            form_row(ui, t, "Initial capital", |ui| {
                number(ui, &mut p.initial_capital, 100.0, 0.01, 1e15, "");
            });
            form_row(ui, t, "Order size", |ui| {
                let step = if p.qty_type == QtyType::Fixed { 0.1 } else { 1.0 };
                number(ui, &mut p.qty_value, step, 0.0, 1e12, "");
                egui::ComboBox::from_id_salt("props-qty")
                    .selected_text(match p.qty_type {
                        QtyType::Fixed => "Contracts",
                        QtyType::Cash => "Cash",
                        QtyType::PercentOfEquity => "% of equity",
                    })
                    .width(110.0)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut p.qty_type, QtyType::Fixed, "Contracts");
                        ui.selectable_value(&mut p.qty_type, QtyType::Cash, "Cash");
                        ui.selectable_value(&mut p.qty_type, QtyType::PercentOfEquity, "% of equity");
                    });
            });
            form_row(ui, t, "Pyramiding", |ui| {
                let mut v = p.pyramiding.max(1) as f64;
                number(ui, &mut v, 0.1, 1.0, 100.0, " orders");
                p.pyramiding = v as u32;
            });
            section(ui, t, "Costs");
            form_row(ui, t, "Commission", |ui| {
                number(ui, &mut p.commission_value, 0.01, 0.0, 1e9, "");
                egui::ComboBox::from_id_salt("props-comm")
                    .selected_text(match p.commission {
                        Commission::Percent => "%",
                        Commission::CashPerContract => "Per contract",
                        Commission::CashPerOrder => "Per order",
                    })
                    .width(110.0)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut p.commission, Commission::Percent, "%");
                        ui.selectable_value(&mut p.commission, Commission::CashPerContract, "Per contract");
                        ui.selectable_value(&mut p.commission, Commission::CashPerOrder, "Per order");
                    });
            });
            form_row(ui, t, "Slippage", |ui| {
                let mut v = p.slippage as f64;
                number(ui, &mut v, 0.1, 0.0, 10_000.0, " ticks");
                p.slippage = v as u32;
            });
            section(ui, t, "Margin");
            form_row(ui, t, "Margin for long positions", |ui| {
                number(ui, &mut p.margin_long, 1.0, 0.0, 100.0, " %");
            });
            form_row(ui, t, "Margin for short positions", |ui| {
                number(ui, &mut p.margin_short, 1.0, 0.0, 100.0, " %");
            });
            section(ui, t, "Fills");
            form_row(ui, t, "Fill orders on bar close", |ui| {
                ui.checkbox(&mut p.process_orders_on_close, "");
            });
            ui.add_space(4.0);
            let changed = p.diff_json(&self.base).as_object().is_some_and(|o| !o.is_empty());
            let note = if changed {
                "Changed from what the script says. Reset goes back to the script's values."
            } else {
                "These are the script's own values."
            };
            ui.label(egui::RichText::new(note).color(t.text3).size(11.5));
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if ui.add_enabled(changed, egui::Button::new("Reset")).clicked() {
                    self.p = self.base.clone();
                }
                ui.add_space(ui.available_width() - 120.0);
                if ui.button("Cancel").clicked() {
                    done = Done::Close;
                }
                if ui.button("OK").clicked() {
                    let over = self.p.diff_json(&self.base);
                    app.set_strategy_overrides(&self.file, over);
                    done = Done::Close;
                }
            });
        }) {
            done = Done::Close;
        }
        done
    }
}

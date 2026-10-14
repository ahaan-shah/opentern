//! The Object tree (everything on the chart: indicators and drawings, with
//! show/hide, lock and remove) and the Data window (values under the
//! crosshair).

use crate::app::App;
use crate::format;
use crate::icons::ph;
use crate::theme::{self, Theme};
use egui::{Align, Layout, Ui};
use opentern_series::study::Format;

impl App {
    pub(crate) fn objects_widget(&mut self, ui: &mut Ui, t: &Theme) {
        egui::ScrollArea::vertical().id_salt("objects").show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(
                egui::RichText::new("OBJECT TREE")
                    .font(theme::semibold(11.0))
                    .color(t.text3),
            );
            ui.label(
                egui::RichText::new(format!("{} · {}", self.chart.symbol.ticker(), self.chart.tf.label()))
                    .font(theme::medium(13.0)),
            );
            if self.chart.studies.is_empty() && self.chart.drawings.is_empty() {
                crate::widgets::empty_state(
                    ui,
                    t,
                    ph::STACK,
                    "Nothing on the chart yet",
                    "Indicators and drawings you add show up here.",
                );
            }
            let mut remove_study = None;
            let mut settings = None;
            let ids: Vec<u64> = self.chart.studies.iter().map(|s| s.uid).collect();
            for uid in ids {
                let Some(s) = self.chart.studies.iter_mut().find(|s| s.uid == uid) else {
                    continue;
                };
                ui.horizontal(|ui| {
                    ui.label(format!("{}  {}", ph::FUNCTION, s.title(true)));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.small_button(ph::X).on_hover_text("Remove").clicked() {
                            remove_study = Some(uid);
                        }
                        if ui.small_button(ph::GEAR_SIX).on_hover_text("Settings").clicked() {
                            settings = Some(uid);
                        }
                        let eye = if s.visible { ph::EYE } else { ph::EYE_SLASH };
                        if ui.small_button(eye).clicked() {
                            s.visible = !s.visible;
                        }
                    });
                });
            }
            if let Some(uid) = remove_study {
                self.chart.remove_study(uid);
            }
            if let Some(uid) = settings {
                self.chart_actions(vec![crate::chart::ui::Action::StudySettings(uid)]);
            }
            let mut remove = None;
            let mut select = None;
            let mut changed = false;
            for d in &mut self.chart.drawings {
                ui.horizontal(|ui| {
                    let on = self.chart_ui.draw.selected == Some(d.id);
                    let text = egui::RichText::new(format!("{}  {}", d.tool.icon(), d.tool.name())).color(if on {
                        t.accent
                    } else {
                        t.text
                    });
                    if ui.add(egui::Button::new(text).frame(false)).clicked() {
                        select = Some(d.id);
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.small_button(ph::X).on_hover_text("Remove").clicked() {
                            remove = Some(d.id);
                        }
                        let lock = if d.locked {
                            ph::LOCK_SIMPLE
                        } else {
                            ph::LOCK_SIMPLE_OPEN
                        };
                        if ui.small_button(lock).clicked() {
                            d.locked = !d.locked;
                            changed = true;
                        }
                        let eye = if d.hidden { ph::EYE_SLASH } else { ph::EYE };
                        if ui.small_button(eye).clicked() {
                            d.hidden = !d.hidden;
                            changed = true;
                        }
                    });
                });
            }
            if changed {
                self.chart.drawings_changed += 1;
            }
            if let Some(id) = select {
                self.chart_ui.draw.selected = Some(id);
            }
            if let Some(id) = remove {
                self.chart.push_undo();
                self.chart.drawings.retain(|d| d.id != id);
                self.chart.drawings_changed += 1;
            }
            ui.add_space(12.0);
            ui.label(
                egui::RichText::new("DATA WINDOW")
                    .font(theme::semibold(11.0))
                    .color(t.text3),
            );
            self.data_window(ui, t);
        });
    }

    fn data_window(&self, ui: &mut Ui, t: &Theme) {
        let c = &self.chart;
        let Some(i) = self.chart_ui.hover_index.filter(|i| *i < c.bars.len()) else {
            ui.label(egui::RichText::new("Move the pointer over the chart.").color(t.text3));
            return;
        };
        let b = c.bars[i];
        let prec = c.precision();
        let row = |ui: &mut Ui, k: &str, v: String| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(k).color(t.text2));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| ui.label(v));
            });
        };
        row(ui, "Date", format::crosshair_time(b.time, c.tf, &c.tz));
        row(ui, "Open", format::price(b.open, prec));
        row(ui, "High", format::price(b.high, prec));
        row(ui, "Low", format::price(b.low, prec));
        row(ui, "Close", format::price(b.close, prec));
        if i > 0 {
            let p = c.bars[i - 1].close;
            row(
                ui,
                "Change",
                format!(
                    "{} ({})",
                    format::price(b.close - p, prec),
                    format::signed_pct((b.close / p - 1.0) * 100.0)
                ),
            );
        }
        row(ui, "Volume", format::compact(b.volume));
        for s in &c.studies {
            ui.add_space(4.0);
            ui.label(egui::RichText::new(s.title(true)).font(theme::medium(12.5)));
            for (pi, spec) in s.meta().plots.iter().enumerate() {
                let off = s.meta().offset(pi, &s.inst.inputs);
                let src = i as i64 - off;
                let v = if src >= 0 && (src as usize) < c.bars.len() {
                    s.inst.out.plots[pi][src as usize]
                } else {
                    f64::NAN
                };
                let text = match s.meta().format {
                    Format::Price => format::price(v, prec),
                    Format::Fixed(d) => format::price(v, d as usize),
                    Format::Volume => format::compact(v),
                };
                row(ui, spec.title, text);
            }
        }
    }
}

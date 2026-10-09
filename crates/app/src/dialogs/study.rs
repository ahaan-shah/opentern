//! An indicator's settings, as on TradingView: Inputs, Style and
//! Visibility tabs, a Defaults menu, and changes previewed live on the
//! chart (Cancel puts things back).

use super::color::color_button;
use super::{Done, form_row, frame, section};
use crate::app::App;
use crate::chart::StudyLook;
use crate::chart::style::UNIT_NAMES;
use crate::icons::ph;
use crate::theme::Theme;
use egui::{Align, Layout, Ui};
use opentern_series::study::{InputKind, Inputs, LineStyle, PlotStyle, Source, Value};

#[derive(Clone, Copy, PartialEq)]
enum Tab {
    Inputs,
    Style,
    Visibility,
}

pub struct StudySettings {
    uid: u64,
    tab: Tab,
    orig: (Inputs, StudyLook),
    inputs: Inputs,
    look: StudyLook,
}

pub fn line_style_name(s: LineStyle) -> &'static str {
    match s {
        LineStyle::Solid => "Solid",
        LineStyle::Dashed => "Dashed",
        LineStyle::Dotted => "Dotted",
    }
}

impl StudySettings {
    pub fn new(app: &App, uid: u64) -> Option<StudySettings> {
        let s = app.chart.studies.iter().find(|s| s.uid == uid)?;
        Some(StudySettings {
            uid,
            tab: if s.meta().inputs.is_empty() {
                Tab::Style
            } else {
                Tab::Inputs
            },
            orig: (s.inst.inputs.clone(), s.look.clone()),
            inputs: s.inst.inputs.clone(),
            look: s.look.clone(),
        })
    }

    pub fn cancel(&self, app: &mut App) {
        app.chart.set_inputs(self.uid, &self.orig.0);
        if let Some(s) = app.chart.study_mut(self.uid) {
            s.look = self.orig.1.clone();
        }
    }

    pub fn show(&mut self, ui: &mut Ui, app: &mut App, t: &Theme) -> Done {
        let Some(meta) = app.chart.studies.iter().find(|s| s.uid == self.uid).map(|s| s.meta()) else {
            return Done::Close;
        };
        let mut done = Done::Keep;
        let mut inputs_changed = false;
        let mut look_changed = false;
        let closed = frame(ui, t, meta.name, 480.0, |ui| {
            ui.horizontal(|ui| {
                for (tab, label) in [
                    (Tab::Inputs, "Inputs"),
                    (Tab::Style, "Style"),
                    (Tab::Visibility, "Visibility"),
                ] {
                    if tab == Tab::Inputs && meta.inputs.is_empty() {
                        continue;
                    }
                    let on = self.tab == tab;
                    let text = egui::RichText::new(label).color(if on { t.accent } else { t.text2 });
                    if ui.add(egui::Button::new(text).selected(on)).clicked() {
                        self.tab = tab;
                    }
                }
            });
            ui.separator();
            egui::ScrollArea::vertical().max_height(420.0).show(ui, |ui| {
                ui.set_width(480.0 - 12.0);
                match self.tab {
                    Tab::Inputs => inputs_changed = self.inputs_tab(ui, t, meta),
                    Tab::Style => look_changed = self.style_tab(ui, t, meta),
                    Tab::Visibility => {
                        section(ui, t, "Show on intervals");
                        for (i, name) in UNIT_NAMES.iter().enumerate() {
                            look_changed |= ui.checkbox(&mut self.look.intervals[i], *name).changed();
                        }
                    }
                }
            });
            ui.separator();
            ui.horizontal(|ui| {
                let resp = ui.button(format!("Defaults  {}", ph::CARET_DOWN));
                egui::Popup::menu(&resp).show(|ui| {
                    let saved = app.store.indicator_default(meta.id).is_some();
                    if ui.button("Reset settings").clicked() {
                        (self.inputs, self.look) = app.study_defaults(meta);
                        inputs_changed = true;
                        look_changed = true;
                        ui.close();
                    }
                    if ui.button("Save as default").clicked() {
                        app.save_study_default(meta, &self.inputs, &self.look);
                        ui.close();
                    }
                    if ui
                        .add_enabled(saved, egui::Button::new("Restore factory defaults"))
                        .clicked()
                    {
                        app.clear_study_default(meta);
                        self.inputs = meta.default_inputs();
                        self.look = StudyLook::defaults(meta);
                        inputs_changed = true;
                        look_changed = true;
                        ui.close();
                    }
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.button("  Ok  ").clicked() {
                        done = Done::Close;
                    }
                    if ui.button("Cancel").clicked() {
                        self.cancel(app);
                        done = Done::Close;
                    }
                });
            });
        });
        if inputs_changed {
            app.chart.set_inputs(self.uid, &self.inputs);
            // The study normalises its inputs; keep the dialog in step.
            if let Some(s) = app.chart.studies.iter().find(|s| s.uid == self.uid) {
                self.inputs = s.inst.inputs.clone();
            }
        }
        if look_changed && let Some(s) = app.chart.study_mut(self.uid) {
            s.look = self.look.clone();
            app.chart.touch();
        }
        if closed {
            self.cancel(app);
            done = Done::Close;
        }
        done
    }

    fn inputs_tab(&mut self, ui: &mut Ui, t: &Theme, meta: &opentern_series::Meta) -> bool {
        let mut changed = false;
        let mut group: Option<&str> = None;
        for spec in &meta.inputs {
            if spec.group != group {
                group = spec.group;
                if let Some(g) = group {
                    section(ui, t, g);
                }
            }
            let mut value = self.inputs.values.get(spec.id).cloned().unwrap_or(spec.default.clone());
            form_row(ui, t, spec.title, |ui| match (&spec.kind, &mut value) {
                (InputKind::Int { min, max }, Value::Int(v)) => {
                    changed |= ui.add(egui::DragValue::new(v).range(*min..=*max).speed(0.2)).changed();
                }
                (InputKind::Float { min, max, step }, Value::Float(v)) => {
                    changed |= ui
                        .add(
                            egui::DragValue::new(v)
                                .range(*min..=*max)
                                .speed(*step * 0.2)
                                .max_decimals(4),
                        )
                        .changed();
                }
                (InputKind::Bool, Value::Bool(v)) => {
                    changed |= ui.checkbox(v, "").changed();
                }
                (InputKind::Source, Value::Text(v)) => {
                    egui::ComboBox::from_id_salt(spec.id)
                        .selected_text(v.as_str())
                        .show_ui(ui, |ui| {
                            for s in Source::ALL {
                                changed |= ui.selectable_value(v, s.name().to_string(), s.name()).changed();
                            }
                        });
                }
                (InputKind::Text, Value::Text(v)) => {
                    changed |= ui.add(egui::TextEdit::singleline(v).desired_width(180.0)).changed();
                }
                (InputKind::Color, Value::Text(v)) => {
                    let mut c = crate::scripting::parse_color(v);
                    if color_button(ui, t, &mut c) {
                        *v = format!("#{:08X}", c.0);
                        changed = true;
                    }
                }
                (InputKind::Choice(opts), Value::Text(v)) => {
                    egui::ComboBox::from_id_salt(spec.id)
                        .width(180.0)
                        .selected_text(v.as_str())
                        .show_ui(ui, |ui| {
                            for o in opts.iter() {
                                changed |= ui.selectable_value(v, o.to_string(), *o).changed();
                            }
                        });
                }
                _ => {}
            });
            self.inputs.set(spec.id, value);
        }
        changed
    }

    fn style_tab(&mut self, ui: &mut Ui, t: &Theme, meta: &opentern_series::Meta) -> bool {
        let mut changed = false;
        for (i, spec) in meta.plots.iter().enumerate() {
            if spec.style == PlotStyle::Hidden {
                continue;
            }
            let fixed_style = matches!(spec.style, PlotStyle::Shape | PlotStyle::Candle | PlotStyle::OhlcBar);
            let look = &mut self.look.plots[i];
            ui.horizontal(|ui| {
                changed |= ui.checkbox(&mut look.visible, spec.title).changed();
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if !fixed_style {
                        egui::ComboBox::from_id_salt(("plot-style", i))
                            .width(110.0)
                            .selected_text(look.style.name())
                            .show_ui(ui, |ui| {
                                for s in PlotStyle::ALL {
                                    changed |= ui.selectable_value(&mut look.style, s, s.name()).changed();
                                }
                            });
                        egui::ComboBox::from_id_salt(("plot-width", i))
                            .width(50.0)
                            .selected_text(format!("{}px", look.width))
                            .show_ui(ui, |ui| {
                                for w in 1..=4u8 {
                                    changed |= ui.selectable_value(&mut look.width, w, format!("{w}px")).changed();
                                }
                            });
                    }
                    for c in look.colors.iter_mut().rev() {
                        changed |= color_button(ui, t, c);
                    }
                });
            });
        }
        if !meta.hlines.is_empty() {
            section(ui, t, "Levels");
        }
        for (i, spec) in meta.hlines.iter().enumerate() {
            let h = &mut self.look.hlines[i];
            ui.horizontal(|ui| {
                changed |= ui.checkbox(&mut h.visible, spec.title).changed();
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    egui::ComboBox::from_id_salt(("hline-style", i))
                        .width(80.0)
                        .selected_text(line_style_name(h.style))
                        .show_ui(ui, |ui| {
                            for s in [LineStyle::Solid, LineStyle::Dashed, LineStyle::Dotted] {
                                changed |= ui.selectable_value(&mut h.style, s, line_style_name(s)).changed();
                            }
                        });
                    changed |= ui
                        .add(egui::DragValue::new(&mut h.value).speed(0.1).max_decimals(4))
                        .changed();
                    changed |= color_button(ui, t, &mut h.color);
                });
            });
        }
        if !meta.fills.is_empty() {
            section(ui, t, "Fills");
        }
        for (i, spec) in meta.fills.iter().enumerate() {
            let f = &mut self.look.fills[i];
            ui.horizontal(|ui| {
                changed |= ui.checkbox(&mut f.visible, spec.title).changed();
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    for c in f.colors.iter_mut().rev() {
                        changed |= color_button(ui, t, c);
                    }
                });
            });
        }
        section(ui, t, "Outputs");
        changed |= ui
            .checkbox(&mut self.look.labels_on_scale, "Labels on price scale")
            .changed();
        form_row(ui, t, "Precision", |ui| {
            let label = self.look.precision.map(|p| p.to_string()).unwrap_or("Default".into());
            egui::ComboBox::from_id_salt("precision")
                .selected_text(label)
                .show_ui(ui, |ui| {
                    changed |= ui.selectable_value(&mut self.look.precision, None, "Default").changed();
                    for p in 0..=8u8 {
                        changed |= ui
                            .selectable_value(&mut self.look.precision, Some(p), p.to_string())
                            .changed();
                    }
                });
        });
        changed
    }
}

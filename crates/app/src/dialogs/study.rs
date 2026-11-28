//! An indicator's settings, as on TradingView: Inputs, Style and
//! Visibility tabs, a Defaults menu, and changes previewed live on the
//! chart (Cancel puts things back).

use super::color::color_button;
use super::{Done, form_row, frame, section};
use crate::app::App;
use crate::chart::StudyLook;
use crate::chart::style::UNIT_NAMES;
use crate::i18n::{nt, of_study, tr, trf};
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
    /// The symbol search open over the dialog, for this input.
    pub(crate) picking: Option<(&'static str, Box<super::SymbolSearch>)>,
}

/// The other indicators' plots study `me`'s `Source` inputs can read
/// ("indicator on indicator"): (link key, name). Never a study that
/// already reads `me`, directly or through others (that would be a loop).
pub(crate) fn link_choices(chart: &crate::chart::Chart, me: u64) -> Vec<(String, String)> {
    if !chart.studies.iter().any(|s| s.uid == me) {
        return Vec::new();
    }
    let readers = chart.readers_of(me);
    chart
        .studies
        .iter()
        .filter(|s| s.uid != me && !readers.contains(&s.uid))
        .flat_map(|s| {
            let meta = s.meta();
            let name = chart.study_title(s, true);
            meta.plots.iter().enumerate().map(move |(i, p)| {
                (
                    opentern_series::study::link_key(s.uid, i),
                    format!("{name} · {}", of_study(meta, p.title)),
                )
            })
        })
        .collect()
}

pub fn line_style_name(s: LineStyle) -> &'static str {
    match s {
        LineStyle::Solid => nt!("Solid"),
        LineStyle::Dashed => nt!("Dashed"),
        LineStyle::Dotted => nt!("Dotted"),
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
            picking: None,
        })
    }

    /// With the symbol search open for `input`.
    pub fn picking(mut self, input: &'static str) -> StudySettings {
        self.tab = Tab::Inputs;
        self.picking = Some((input, Box::new(super::SymbolSearch::for_input())));
        self
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
        // A symbol input's search, in place of the settings until it's done
        // (Esc or its X goes back to them).
        if let Some((id, search)) = &mut self.picking {
            let back = ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
            let done = search.show(ui, app, t);
            if let Some(sym) = search.picked.take() {
                self.inputs.set(id, Value::Text(sym.id.clone()));
                app.chart.set_inputs(self.uid, &self.inputs);
                if let Some(s) = app.chart.studies.iter().find(|s| s.uid == self.uid) {
                    self.inputs = s.inst.inputs.clone();
                }
                self.picking = None;
            } else if back || matches!(done, Done::Close) {
                self.picking = None;
            }
            return Done::Keep;
        }
        let links = link_choices(&app.chart, self.uid);
        let mut done = Done::Keep;
        let mut inputs_changed = false;
        let mut look_changed = false;
        let closed = frame(ui, t, of_study(meta, meta.name), 480.0, |ui| {
            let mut tabs = vec![
                (Tab::Inputs, tr("Inputs")),
                (Tab::Style, tr("Style")),
                (Tab::Visibility, tr("Visibility")),
            ];
            if meta.inputs.is_empty() {
                tabs.remove(0);
            }
            crate::widgets::segmented(ui, t, &mut self.tab, &tabs);
            ui.add_space(4.0);
            ui.separator();
            egui::ScrollArea::vertical().max_height(420.0).show(ui, |ui| {
                ui.set_width(480.0 - 12.0);
                match self.tab {
                    Tab::Inputs => inputs_changed = self.inputs_tab(ui, t, meta, &links),
                    Tab::Style => look_changed = self.style_tab(ui, t, meta),
                    Tab::Visibility => {
                        section(ui, t, tr("Show on intervals"));
                        for (i, name) in UNIT_NAMES.iter().enumerate() {
                            look_changed |= crate::widgets::checkbox(ui, &mut self.look.intervals[i], *name).changed();
                        }
                    }
                }
            });
            ui.separator();
            ui.horizontal(|ui| {
                let resp = crate::widgets::button(
                    ui,
                    crate::widgets::Kind::Secondary,
                    true,
                    None,
                    &trf!("Defaults  {}", ph::CARET_DOWN),
                );
                egui::Popup::menu(&resp).show(|ui| {
                    ui.set_width(230.0);
                    let saved = app.store.indicator_default(meta.id).is_some();
                    if crate::widgets::menu_item(
                        ui,
                        t,
                        Some(ph::ARROW_COUNTER_CLOCKWISE),
                        tr("Reset settings"),
                        "",
                        false,
                    )
                    .clicked()
                    {
                        (self.inputs, self.look) = app.study_defaults(meta);
                        inputs_changed = true;
                        look_changed = true;
                        ui.close();
                    }
                    if crate::widgets::menu_item(ui, t, Some(ph::FLOPPY_DISK), tr("Save as default"), "", false)
                        .clicked()
                    {
                        app.save_study_default(meta, &self.inputs, &self.look);
                        ui.close();
                    }
                    if crate::widgets::menu_row(
                        ui,
                        t,
                        Some(ph::FACTORY),
                        tr("Restore factory defaults"),
                        "",
                        false,
                        saved,
                    )
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
                    if crate::widgets::primary(ui, tr("Ok")).clicked() {
                        done = Done::Close;
                    }
                    if crate::widgets::secondary(ui, tr("Cancel")).clicked() {
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

    fn inputs_tab(&mut self, ui: &mut Ui, t: &Theme, meta: &opentern_series::Meta, links: &[(String, String)]) -> bool {
        let mut changed = false;
        let mut pick: Option<&'static str> = None;
        let mut group: Option<&str> = None;
        for spec in &meta.inputs {
            if spec.group != group {
                group = spec.group;
                if let Some(g) = group {
                    section(ui, t, of_study(meta, g));
                }
            }
            let mut value = self.inputs.values.get(spec.id).cloned().unwrap_or(spec.default.clone());
            form_row(ui, t, of_study(meta, spec.title), |ui| match (&spec.kind, &mut value) {
                (InputKind::Int { min, max, step }, Value::Int(v)) => {
                    let mut x = *v as f64;
                    let id = ui.id().with(("input", spec.id));
                    if crate::widgets::number(ui, id, &mut x, *step as f64, *min as f64, *max as f64, 0) {
                        *v = x as i64;
                        changed = true;
                    }
                }
                (InputKind::Float { min, max, step }, Value::Float(v)) => {
                    let places = crate::widgets::decimals_of(*step)
                        .max(crate::widgets::decimals_of(*v))
                        .max(1);
                    let id = ui.id().with(("input", spec.id));
                    changed |= crate::widgets::number(ui, id, v, *step, *min, *max, places);
                }
                (InputKind::Bool, Value::Bool(v)) => {
                    changed |= crate::widgets::checkbox(ui, v, "").changed();
                }
                (InputKind::Source, Value::Text(v)) => {
                    // A linked source shows the indicator and plot it reads.
                    let shown = match links.iter().find(|(key, _)| key == v) {
                        Some((_, name)) => name.clone(),
                        None if opentern_series::study::parse_link(v).is_some() => "—".to_string(),
                        None => tr(v).to_string(),
                    };
                    crate::widgets::combo(spec.id).selected_text(shown).show_ui(ui, |ui| {
                        for s in Source::ALL {
                            changed |=
                                crate::widgets::option_value(ui, v, s.name().to_string(), tr(s.name())).changed();
                        }
                        for (key, name) in links {
                            changed |= crate::widgets::option_value(ui, v, key.clone(), name.as_str()).changed();
                        }
                    });
                }
                (InputKind::Text, Value::Text(v)) => {
                    changed |= ui.add(crate::widgets::field(v).desired_width(180.0)).changed();
                }
                (InputKind::Symbol { .. }, Value::Text(v)) => {
                    let shown = if v.is_empty() { "—" } else { v.as_str() };
                    let r = crate::widgets::button(
                        ui,
                        crate::widgets::Kind::Secondary,
                        true,
                        Some(ph::MAGNIFYING_GLASS),
                        shown,
                    );
                    if r.clicked() {
                        pick = Some(spec.id);
                    }
                }
                (InputKind::Color, Value::Text(v)) => {
                    let mut c = crate::scripting::parse_color(v);
                    if color_button(ui, t, &mut c) {
                        *v = format!("#{:08X}", c.0);
                        changed = true;
                    }
                }
                (InputKind::Choice(opts), Value::Text(v)) => {
                    crate::widgets::combo(spec.id)
                        .width(180.0)
                        .selected_text(of_study(meta, v))
                        .show_ui(ui, |ui| {
                            for o in opts.iter() {
                                changed |=
                                    crate::widgets::option_value(ui, v, o.to_string(), of_study(meta, o)).changed();
                            }
                        });
                }
                _ => {}
            });
            self.inputs.set(spec.id, value);
        }
        if let Some(id) = pick {
            self.picking = Some((id, Box::new(super::SymbolSearch::for_input())));
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
                changed |= crate::widgets::checkbox(ui, &mut look.visible, of_study(meta, spec.title)).changed();
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if !fixed_style {
                        crate::widgets::combo(("plot-style", i))
                            .width(110.0)
                            .selected_text(tr(look.style.name()))
                            .show_ui(ui, |ui| {
                                for s in PlotStyle::ALL {
                                    changed |=
                                        crate::widgets::option_value(ui, &mut look.style, s, tr(s.name())).changed();
                                }
                            });
                        crate::widgets::combo(("plot-width", i))
                            .width(50.0)
                            .selected_text(trf!("{}px", look.width))
                            .show_ui(ui, |ui| {
                                for w in 1..=4u8 {
                                    changed |=
                                        crate::widgets::option_value(ui, &mut look.width, w, trf!("{w}px")).changed();
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
            section(ui, t, tr("Levels"));
        }
        for (i, spec) in meta.hlines.iter().enumerate() {
            let h = &mut self.look.hlines[i];
            ui.horizontal(|ui| {
                changed |= crate::widgets::checkbox(ui, &mut h.visible, of_study(meta, spec.title)).changed();
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    crate::widgets::combo(("hline-style", i))
                        .width(80.0)
                        .selected_text(line_style_name(h.style))
                        .show_ui(ui, |ui| {
                            for s in [LineStyle::Solid, LineStyle::Dashed, LineStyle::Dotted] {
                                changed |=
                                    crate::widgets::option_value(ui, &mut h.style, s, line_style_name(s)).changed();
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
            section(ui, t, tr("Fills"));
        }
        for (i, spec) in meta.fills.iter().enumerate() {
            let f = &mut self.look.fills[i];
            ui.horizontal(|ui| {
                changed |= crate::widgets::checkbox(ui, &mut f.visible, of_study(meta, spec.title)).changed();
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    for c in f.colors.iter_mut().rev() {
                        changed |= color_button(ui, t, c);
                    }
                });
            });
        }
        section(ui, t, tr("Outputs"));
        changed |= crate::widgets::checkbox(ui, &mut self.look.labels_on_scale, tr("Labels on price scale")).changed();
        form_row(ui, t, tr("Precision"), |ui| {
            let label = self
                .look
                .precision
                .map(|p| p.to_string())
                .unwrap_or(tr("Default").into());
            crate::widgets::combo("precision")
                .selected_text(label)
                .show_ui(ui, |ui| {
                    changed |=
                        crate::widgets::option_value(ui, &mut self.look.precision, None, tr("Default")).changed();
                    for p in 0..=8u8 {
                        changed |= crate::widgets::option_value(ui, &mut self.look.precision, Some(p), p.to_string())
                            .changed();
                    }
                });
        });
        changed
    }
}

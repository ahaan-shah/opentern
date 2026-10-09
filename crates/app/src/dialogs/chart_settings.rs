//! Chart settings, with TradingView's tabs (Symbol, Status line, Scales and
//! lines, Canvas) and a Template menu to save, apply and set defaults.
//! Changes preview live; Cancel puts them back.

use super::color::color_button;
use super::study::line_style_name;
use super::{Done, TemplateKind, form_row, frame, section};
use crate::app::App;
use crate::chart::ChartStyle;
use crate::chart::style::{SeriesKind, TimeZoneChoice, c32, rgba};
use crate::icons::ph;
use crate::motion;
use crate::theme::{self, Theme};
use egui::{Align, Align2, CornerRadius, Layout, Sense, Ui, pos2, vec2};
use opentern_series::Rgba;
use opentern_series::study::LineStyle;

#[derive(Clone, Copy, PartialEq)]
enum Tab {
    Symbol,
    Status,
    Scales,
    Canvas,
}

pub struct ChartSettings {
    tab: Tab,
    orig: ChartStyle,
    style: ChartStyle,
}

impl ChartSettings {
    pub fn new(app: &App) -> ChartSettings {
        ChartSettings {
            tab: Tab::Symbol,
            orig: app.chart.style.clone(),
            style: app.chart.style.clone(),
        }
    }

    pub fn cancel(&self, app: &mut App) {
        app.set_chart_style(self.orig.clone());
    }

    pub fn show(&mut self, ui: &mut Ui, app: &mut App, t: &Theme) -> Done {
        let mut done = Done::Keep;
        let mut changed = false;
        let closed = frame(ui, t, "Chart settings", 640.0, |ui| {
            let height = 440.0;
            ui.horizontal_top(|ui| {
                ui.allocate_ui_with_layout(vec2(160.0, height), Layout::top_down(Align::Min), |ui| {
                    ui.set_min_size(vec2(160.0, height));
                    for (tab, icon, label) in [
                        (Tab::Symbol, ph::CHART_BAR, "Symbol"),
                        (Tab::Status, ph::LIST_DASHES, "Status line"),
                        (Tab::Scales, ph::RULER, "Scales and lines"),
                        (Tab::Canvas, ph::PAINT_BRUSH, "Canvas"),
                    ] {
                        let on = self.tab == tab;
                        let (rect, resp) = ui.allocate_exact_size(vec2(160.0, 32.0), Sense::click());
                        let hov = motion::toggle(ui.ctx(), resp.id, resp.hovered(), motion::MICRO);
                        let bg = if on {
                            t.accent_soft()
                        } else {
                            motion::with_alpha(t.hover_wash(), hov)
                        };
                        ui.painter().rect_filled(rect, CornerRadius::same(7), bg);
                        ui.painter().text(
                            pos2(rect.left() + 10.0, rect.center().y),
                            Align2::LEFT_CENTER,
                            format!("{icon}  {label}"),
                            theme::medium(13.0),
                            if on { t.accent } else { t.text },
                        );
                        if resp.clicked() {
                            self.tab = tab;
                        }
                    }
                });
                // A painted divider: `ui.separator()` in a row takes all
                // the height it's offered, which in a modal is unbounded.
                let (line, _) = ui.allocate_exact_size(vec2(17.0, height), Sense::hover());
                ui.painter()
                    .vline(line.center().x, line.y_range(), egui::Stroke::new(1.0, t.border));
                let w = ui.available_width();
                ui.allocate_ui_with_layout(vec2(w, height), Layout::top_down(Align::Min), |ui| {
                    egui::ScrollArea::vertical().max_height(height).show(ui, |ui| {
                        ui.set_width(w - 12.0);
                        changed = match self.tab {
                            Tab::Symbol => symbol_tab(ui, t, &mut self.style),
                            Tab::Status => status_tab(ui, &mut self.style),
                            Tab::Scales => scales_tab(ui, t, &mut self.style),
                            Tab::Canvas => canvas_tab(ui, t, &mut self.style),
                        };
                    });
                });
            });
            ui.separator();
            ui.horizontal(|ui| {
                let resp = ui.button(format!("Template  {}", ph::CARET_DOWN));
                egui::Popup::menu(&resp).show(|ui| {
                    ui.set_min_width(220.0);
                    if ui.button("Save as…").clicked() {
                        app.set_chart_style(self.style.clone());
                        app.modal = Some(super::Modal::SaveTemplate {
                            kind: TemplateKind::Chart,
                            name: String::new(),
                        });
                        ui.close();
                    }
                    if ui.button("Apply defaults").clicked() {
                        self.style = ChartStyle::default();
                        changed = true;
                        ui.close();
                    }
                    let templates = app.store.templates("chart").to_vec();
                    if !templates.is_empty() {
                        ui.separator();
                    }
                    for tpl in templates {
                        ui.horizontal(|ui| {
                            let label = if tpl.is_default {
                                format!("{}  (default)", tpl.name)
                            } else {
                                tpl.name.clone()
                            };
                            if ui.button(label).clicked()
                                && let Ok(s) = serde_json::from_str::<ChartStyle>(&tpl.json)
                            {
                                self.style = s;
                                changed = true;
                                ui.close();
                            }
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                if ui.small_button(ph::TRASH).on_hover_text("Delete").clicked() {
                                    app.delete_template(TemplateKind::Chart, &tpl.name);
                                }
                                let tip = if tpl.is_default {
                                    "Stop using as default"
                                } else {
                                    "Use for new charts"
                                };
                                let star = if tpl.is_default { "★" } else { "☆" };
                                if ui.small_button(star).on_hover_text(tip).clicked() {
                                    app.set_default_template(
                                        TemplateKind::Chart,
                                        (!tpl.is_default).then_some(tpl.name.as_str()),
                                    );
                                }
                            });
                        });
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
        if changed {
            app.set_chart_style(self.style.clone());
        }
        if closed {
            self.cancel(app);
            done = Done::Close;
        }
        done
    }
}

/// A colour that follows the theme until set; shows the theme colour.
fn theme_color(ui: &mut Ui, t: &Theme, c: &mut Option<Rgba>, fallback: egui::Color32) -> bool {
    let mut v = c.unwrap_or(rgba(fallback));
    let mut changed = color_button(ui, t, &mut v);
    if changed {
        *c = Some(v);
    }
    if c.is_some() && ui.small_button("Theme").on_hover_text("Follow the app theme").clicked() {
        *c = None;
        changed = true;
    }
    changed
}

fn up_down(ui: &mut Ui, t: &Theme, label: &str, up: &mut Rgba, down: &mut Rgba, show: Option<&mut bool>) -> bool {
    let mut changed = false;
    form_row(ui, t, label, |ui| {
        if let Some(s) = show {
            changed |= ui.checkbox(s, "").changed();
        }
        changed |= color_button(ui, t, up);
        changed |= color_button(ui, t, down);
    });
    changed
}

fn symbol_tab(ui: &mut Ui, t: &Theme, s: &mut ChartStyle) -> bool {
    let mut changed = false;
    section(ui, t, "Chart type");
    form_row(ui, t, "Type", |ui| {
        egui::ComboBox::from_id_salt("series-kind")
            .width(160.0)
            .selected_text(s.kind.name())
            .show_ui(ui, |ui| {
                for k in SeriesKind::ALL {
                    changed |= ui.selectable_value(&mut s.kind, k, k.name()).changed();
                }
            });
    });
    match s.kind {
        SeriesKind::Line | SeriesKind::Area => {
            form_row(ui, t, "Line", |ui| {
                changed |= color_button(ui, t, &mut s.line_color);
                egui::ComboBox::from_id_salt("line-width")
                    .width(60.0)
                    .selected_text(format!("{}px", s.line_width))
                    .show_ui(ui, |ui| {
                        for w in 1..=4u8 {
                            changed |= ui.selectable_value(&mut s.line_width, w, format!("{w}px")).changed();
                        }
                    });
            });
            if s.kind == SeriesKind::Area {
                form_row(ui, t, "Fill", |ui| {
                    changed |= color_button(ui, t, &mut s.area_top);
                    changed |= color_button(ui, t, &mut s.area_bottom);
                });
            }
        }
        _ => {
            changed |= ui
                .checkbox(&mut s.color_on_prev_close, "Color bars based on previous close")
                .changed();
            changed |= up_down(ui, t, "Body", &mut s.up_body, &mut s.down_body, Some(&mut s.show_body));
            changed |= up_down(
                ui,
                t,
                "Borders",
                &mut s.up_border,
                &mut s.down_border,
                Some(&mut s.show_borders),
            );
            changed |= up_down(ui, t, "Wick", &mut s.up_wick, &mut s.down_wick, Some(&mut s.show_wick));
        }
    }
    section(ui, t, "Data modification");
    form_row(ui, t, "Precision", |ui| {
        let label = s.precision.map(|p| p.to_string()).unwrap_or("Default".into());
        egui::ComboBox::from_id_salt("chart-precision")
            .selected_text(label)
            .show_ui(ui, |ui| {
                changed |= ui.selectable_value(&mut s.precision, None, "Default").changed();
                for p in 0..=8u8 {
                    changed |= ui.selectable_value(&mut s.precision, Some(p), p.to_string()).changed();
                }
            });
    });
    form_row(ui, t, "Timezone", |ui| {
        let name = |z: TimeZoneChoice| match z {
            TimeZoneChoice::Exchange => "Exchange",
            TimeZoneChoice::Utc => "UTC",
            TimeZoneChoice::Local => "Local",
        };
        egui::ComboBox::from_id_salt("chart-tz")
            .selected_text(name(s.timezone))
            .show_ui(ui, |ui| {
                for z in [TimeZoneChoice::Exchange, TimeZoneChoice::Utc, TimeZoneChoice::Local] {
                    changed |= ui.selectable_value(&mut s.timezone, z, name(z)).changed();
                }
            });
    });
    changed
}

fn status_tab(ui: &mut Ui, s: &mut ChartStyle) -> bool {
    let mut changed = false;
    for (v, label) in [
        (&mut s.show_title, "Title"),
        (&mut s.show_ohlc, "Open, high, low and close values"),
        (&mut s.show_bar_change, "Bar change values"),
        (&mut s.show_volume, "Volume"),
        (&mut s.show_study_titles, "Indicator titles"),
        (&mut s.show_study_inputs, "Indicator inputs"),
        (&mut s.show_study_values, "Indicator values"),
    ] {
        changed |= ui.checkbox(v, label).changed();
    }
    changed
}

fn scales_tab(ui: &mut Ui, t: &Theme, s: &mut ChartStyle) -> bool {
    let mut changed = false;
    section(ui, t, "Price scale");
    for (v, label) in [
        (&mut s.last_price_label, "Symbol last price label"),
        (&mut s.last_price_line, "Symbol last price line"),
        (&mut s.countdown, "Countdown to bar close"),
        (&mut s.study_labels, "Indicator value labels"),
    ] {
        changed |= ui.checkbox(v, label).changed();
    }
    section(ui, t, "Text");
    form_row(ui, t, "Scales text", |ui| {
        changed |= theme_color(ui, t, &mut s.scale_text, t.text2);
        egui::ComboBox::from_id_salt("scale-font")
            .width(60.0)
            .selected_text(s.scale_font.to_string())
            .show_ui(ui, |ui| {
                for f in [10u8, 11, 12, 13, 14, 16] {
                    changed |= ui.selectable_value(&mut s.scale_font, f, f.to_string()).changed();
                }
            });
    });
    changed
}

fn canvas_tab(ui: &mut Ui, t: &Theme, s: &mut ChartStyle) -> bool {
    let mut changed = false;
    let pal = s.palette(t);
    section(ui, t, "Chart basic styles");
    form_row(ui, t, "Background", |ui| {
        changed |= theme_color(ui, t, &mut s.background, pal.bg);
    });
    form_row(ui, t, "Vert grid lines", |ui| {
        changed |= ui.checkbox(&mut s.show_grid_vert, "").changed();
        changed |= theme_color(ui, t, &mut s.grid_vert, pal.grid_v);
    });
    form_row(ui, t, "Horz grid lines", |ui| {
        changed |= ui.checkbox(&mut s.show_grid_horz, "").changed();
        changed |= theme_color(ui, t, &mut s.grid_horz, pal.grid_h);
    });
    form_row(ui, t, "Crosshair", |ui| {
        changed |= theme_color(ui, t, &mut s.crosshair, pal.crosshair);
        egui::ComboBox::from_id_salt("crosshair-style")
            .width(80.0)
            .selected_text(line_style_name(s.crosshair_style))
            .show_ui(ui, |ui| {
                for st in [LineStyle::Solid, LineStyle::Dashed, LineStyle::Dotted] {
                    changed |= ui
                        .selectable_value(&mut s.crosshair_style, st, line_style_name(st))
                        .changed();
                }
            });
    });
    changed |= ui.checkbox(&mut s.watermark, "Symbol watermark").changed();
    let _ = c32;
    changed
}

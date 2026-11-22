//! Chart settings, with TradingView's tabs (Symbol, Status line, Scales and
//! lines, Canvas) and a Template menu to save, apply and set defaults.
//! Changes preview live; Cancel puts them back.

use super::color::localized_color_button as color_button;
use super::study::line_style_name;
use super::{Done, TemplateKind, form_row, localized_frame, section};
use crate::app::App;
use crate::chart::ChartStyle;
use crate::chart::style::{SeriesKind, c32, rgba};
use crate::i18n::Language;
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

    /// The Template button and its menu (save, defaults, saved ones).
    /// Returns true when it changed the style.
    fn template_menu(&mut self, ui: &mut Ui, app: &mut App, t: &Theme) -> bool {
        let lang = app.language;
        let mut changed = false;
        let resp = crate::widgets::button(
            ui,
            crate::widgets::Kind::Secondary,
            true,
            None,
            &format!("{}  {}", lang.text("Template"), ph::CARET_DOWN),
        );
        egui::Popup::menu(&resp).show(|ui| {
            use crate::widgets as w;
            ui.set_width(250.0);
            ui.spacing_mut().item_spacing.y = 1.0;
            if w::menu_item(ui, t, Some(ph::FLOPPY_DISK), lang.text("Save as"), "", false).clicked() {
                app.set_chart_style(self.style.clone());
                app.modal = Some(super::Modal::SaveTemplate {
                    kind: TemplateKind::Chart,
                    name: String::new(),
                });
                ui.close();
            }
            if w::menu_item(
                ui,
                t,
                Some(ph::ARROW_COUNTER_CLOCKWISE),
                lang.text("Apply defaults"),
                "",
                false,
            )
            .clicked()
            {
                self.style = ChartStyle::default();
                changed = true;
                ui.close();
            }
            let templates = app.store.templates("chart").to_vec();
            if !templates.is_empty() {
                w::menu_rule(ui, t);
                w::menu_heading(ui, t, lang.text("SAVED"));
            }
            for tpl in templates {
                ui.horizontal(|ui| {
                    let label = if tpl.is_default {
                        lang.format("{name}  (default)", &[("name", &tpl.name)])
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
                        if ui.small_button(ph::TRASH).on_hover_text(lang.text("Delete")).clicked() {
                            app.delete_template(TemplateKind::Chart, &tpl.name);
                        }
                        let tip = if tpl.is_default {
                            "Stop using as default"
                        } else {
                            "Use for new charts"
                        };
                        let star = if tpl.is_default { "★" } else { "☆" };
                        if ui.small_button(star).on_hover_text(lang.text(tip)).clicked() {
                            app.set_default_template(
                                TemplateKind::Chart,
                                (!tpl.is_default).then_some(tpl.name.as_str()),
                            );
                        }
                    });
                });
            }
        });
        changed
    }

    /// The Settings page's version: tabs as a segmented control, changes
    /// applied as they're made.
    pub fn page(&mut self, ui: &mut Ui, app: &mut App, t: &Theme) {
        let lang = app.language;
        if app.chart.style != self.style && !ui.ctx().egui_is_using_pointer() {
            // Changed elsewhere (a template, the right-click menu): follow it.
            self.style = app.chart.style.clone();
        }
        // Short names when the page is narrow, so all four fit.
        let narrow = ui.available_width() < 560.0;
        crate::widgets::segmented(
            ui,
            t,
            &mut self.tab,
            &[
                (Tab::Symbol, lang.text("Symbol")),
                (
                    Tab::Status,
                    if narrow {
                        lang.text("Status")
                    } else {
                        lang.text("Status line")
                    },
                ),
                (
                    Tab::Scales,
                    if narrow {
                        lang.text("Scales")
                    } else {
                        lang.text("Scales and lines")
                    },
                ),
                (Tab::Canvas, lang.text("Canvas")),
            ],
        );
        ui.add_space(10.0);
        let mut changed = match self.tab {
            Tab::Symbol => symbol_tab(ui, lang, t, &mut self.style, &app.chart.sym_tz),
            Tab::Status => status_tab(ui, lang, &mut self.style),
            Tab::Scales => scales_tab(ui, lang, t, &mut self.style),
            Tab::Canvas => canvas_tab(ui, lang, t, &mut self.style),
        };
        ui.add_space(14.0);
        ui.horizontal(|ui| {
            if self.template_menu(ui, app, t) {
                changed = true;
            }
        });
        if changed {
            app.set_chart_style(self.style.clone());
        }
    }

    pub fn show(&mut self, ui: &mut Ui, app: &mut App, t: &Theme) -> Done {
        let lang = app.language;
        let mut done = Done::Keep;
        let mut changed = false;
        let closed = localized_frame(ui, t, lang, lang.text("Chart settings"), 640.0, |ui| {
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
                            format!("{icon}  {}", lang.text(label)),
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
                            Tab::Symbol => symbol_tab(ui, lang, t, &mut self.style, &app.chart.sym_tz),
                            Tab::Status => status_tab(ui, lang, &mut self.style),
                            Tab::Scales => scales_tab(ui, lang, t, &mut self.style),
                            Tab::Canvas => canvas_tab(ui, lang, t, &mut self.style),
                        };
                    });
                });
            });
            ui.separator();
            ui.horizontal(|ui| {
                if self.template_menu(ui, app, t) {
                    changed = true;
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if crate::widgets::primary(ui, lang.text("Ok")).clicked() {
                        done = Done::Close;
                    }
                    if crate::widgets::secondary(ui, lang.text("Cancel")).clicked() {
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
fn theme_color(ui: &mut Ui, lang: Language, t: &Theme, c: &mut Option<Rgba>, fallback: egui::Color32) -> bool {
    let mut v = c.unwrap_or(rgba(fallback));
    let mut changed = color_button(ui, lang, t, &mut v);
    if changed {
        *c = Some(v);
    }
    if c.is_some()
        && ui
            .small_button(lang.text("Theme"))
            .on_hover_text(lang.text("Follow the app theme"))
            .clicked()
    {
        *c = None;
        changed = true;
    }
    changed
}

fn up_down(
    ui: &mut Ui,
    lang: Language,
    t: &Theme,
    label: &str,
    up: &mut Rgba,
    down: &mut Rgba,
    show: Option<&mut bool>,
) -> bool {
    let mut changed = false;
    form_row(ui, t, label, |ui| {
        if let Some(s) = show {
            changed |= crate::widgets::checkbox(ui, s, "").changed();
        }
        changed |= color_button(ui, lang, t, up);
        changed |= color_button(ui, lang, t, down);
    });
    changed
}

fn symbol_tab(ui: &mut Ui, lang: Language, t: &Theme, s: &mut ChartStyle, exchange: &jiff::tz::TimeZone) -> bool {
    let mut changed = false;
    section(ui, t, lang.text("Chart type"));
    form_row(ui, t, lang.text("Type"), |ui| {
        crate::widgets::combo("series-kind")
            .width(160.0)
            .selected_text(lang.text(s.kind.name()))
            .show_ui(ui, |ui| {
                for k in SeriesKind::ALL {
                    changed |= crate::widgets::option_value(ui, &mut s.kind, k, lang.text(k.name())).changed();
                }
            });
    });
    match s.kind {
        SeriesKind::Baseline => {
            form_row(ui, t, lang.text("Colours"), |ui| {
                changed |= color_button(ui, lang, t, &mut s.baseline_up);
                changed |= color_button(ui, lang, t, &mut s.baseline_down);
            });
            form_row(ui, t, lang.text("Base level"), |ui| {
                changed |= ui
                    .add(egui::Slider::new(&mut s.baseline_level, 0..=100).suffix("%"))
                    .changed();
            });
        }
        SeriesKind::HighLow => {
            form_row(ui, t, lang.text("Bars"), |ui| {
                changed |= color_button(ui, lang, t, &mut s.line_color);
            });
        }
        SeriesKind::Line | SeriesKind::LineMarkers | SeriesKind::StepLine | SeriesKind::Area => {
            form_row(ui, t, lang.text("Line"), |ui| {
                changed |= color_button(ui, lang, t, &mut s.line_color);
                crate::widgets::combo("line-width")
                    .width(60.0)
                    .selected_text(format!("{}px", s.line_width))
                    .show_ui(ui, |ui| {
                        for w in 1..=4u8 {
                            changed |=
                                crate::widgets::option_value(ui, &mut s.line_width, w, format!("{w}px")).changed();
                        }
                    });
            });
            if s.kind == SeriesKind::Area {
                form_row(ui, t, lang.text("Fill"), |ui| {
                    changed |= color_button(ui, lang, t, &mut s.area_top);
                    changed |= color_button(ui, lang, t, &mut s.area_bottom);
                });
            }
        }
        _ => {
            changed |= crate::widgets::checkbox(
                ui,
                &mut s.color_on_prev_close,
                lang.text("Color bars based on previous close"),
            )
            .changed();
            changed |= up_down(
                ui,
                lang,
                t,
                lang.text("Body"),
                &mut s.up_body,
                &mut s.down_body,
                Some(&mut s.show_body),
            );
            changed |= up_down(
                ui,
                lang,
                t,
                lang.text("Borders"),
                &mut s.up_border,
                &mut s.down_border,
                Some(&mut s.show_borders),
            );
            changed |= up_down(
                ui,
                lang,
                t,
                lang.text("Wick"),
                &mut s.up_wick,
                &mut s.down_wick,
                Some(&mut s.show_wick),
            );
        }
    }
    if matches!(s.kind, SeriesKind::Renko | SeriesKind::Kagi | SeriesKind::PointFigure) {
        section(
            ui,
            t,
            if s.kind == SeriesKind::Kagi {
                lang.text("Reversal amount")
            } else {
                lang.text("Box size")
            },
        );
        form_row(ui, t, lang.text("Method"), |ui| {
            let mut atr = s.box_size.is_none();
            crate::widgets::combo("box-method")
                .selected_text(if atr { "ATR" } else { lang.text("Traditional") })
                .show_ui(ui, |ui| {
                    changed |= crate::widgets::option_value(ui, &mut atr, true, "ATR").changed();
                    changed |= crate::widgets::option_value(ui, &mut atr, false, lang.text("Traditional")).changed();
                });
            if atr && s.box_size.is_some() {
                s.box_size = None;
            } else if !atr && s.box_size.is_none() {
                s.box_size = Some(1.0);
            }
        });
        match s.box_size.as_mut() {
            None => form_row(ui, t, lang.text("ATR length"), |ui| {
                changed |= ui.add(egui::DragValue::new(&mut s.atr_length).range(1..=500)).changed();
            }),
            Some(v) => form_row(ui, t, lang.text("Size"), |ui| {
                changed |= ui
                    .add(egui::DragValue::new(v).range(1e-8..=1e9).speed(0.1).max_decimals(8))
                    .changed();
            }),
        }
        if s.kind == SeriesKind::PointFigure {
            form_row(ui, t, lang.text("Reversal"), |ui| {
                changed |= ui
                    .add(egui::DragValue::new(&mut s.pnf_reversal).range(1..=20))
                    .changed();
            });
        }
    }
    if s.kind == SeriesKind::LineBreak {
        section(ui, t, lang.text("Line break"));
        form_row(ui, t, lang.text("Number of lines"), |ui| {
            changed |= ui.add(egui::DragValue::new(&mut s.line_break).range(1..=10)).changed();
        });
    }
    section(ui, t, lang.text("Data modification"));
    form_row(ui, t, lang.text("Precision"), |ui| {
        let label = s
            .precision
            .map(|p| p.to_string())
            .unwrap_or(lang.text("Default").into());
        crate::widgets::combo("chart-precision")
            .selected_text(label)
            .show_ui(ui, |ui| {
                changed |= crate::widgets::option_value(ui, &mut s.precision, None, lang.text("Default")).changed();
                for p in 0..=8u8 {
                    changed |= crate::widgets::option_value(ui, &mut s.precision, Some(p), p.to_string()).changed();
                }
            });
    });
    form_row(ui, t, lang.text("Timezone"), |ui| {
        let resp = crate::widgets::button(
            ui,
            crate::widgets::Kind::Secondary,
            true,
            None,
            &format!(
                "{}  {}",
                lang.text(&crate::timezones::choice_name(&s.timezone)),
                ph::CARET_DOWN
            ),
        );
        egui::Popup::menu(&resp)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .show(|ui| {
                ui.set_width(250.0);
                changed |=
                    crate::timezones::picker(ui, t, lang, egui::Id::new("chart-tz-search"), &mut s.timezone, exchange);
            });
    });
    changed
}

fn status_tab(ui: &mut Ui, lang: Language, s: &mut ChartStyle) -> bool {
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
        changed |= crate::widgets::checkbox(ui, v, lang.text(label)).changed();
    }
    changed
}

fn scales_tab(ui: &mut Ui, lang: Language, t: &Theme, s: &mut ChartStyle) -> bool {
    let mut changed = false;
    section(ui, t, lang.text("Price scale"));
    for (v, label) in [
        (&mut s.last_price_label, "Symbol last price label"),
        (&mut s.last_price_line, "Symbol last price line"),
        (&mut s.countdown, "Countdown to bar close"),
        (&mut s.ext_price_label, "Pre/post market price label"),
        (&mut s.ext_price_line, "Pre/post market price line"),
        (&mut s.study_labels, "Indicator value labels"),
    ] {
        changed |= crate::widgets::checkbox(ui, v, lang.text(label)).changed();
    }
    section(ui, t, lang.text("Text"));
    form_row(ui, t, lang.text("Scales text"), |ui| {
        changed |= theme_color(ui, lang, t, &mut s.scale_text, t.text2);
        crate::widgets::combo("scale-font")
            .width(60.0)
            .selected_text(s.scale_font.to_string())
            .show_ui(ui, |ui| {
                for f in [10u8, 11, 12, 13, 14, 16] {
                    changed |= crate::widgets::option_value(ui, &mut s.scale_font, f, f.to_string()).changed();
                }
            });
    });
    changed
}

fn canvas_tab(ui: &mut Ui, lang: Language, t: &Theme, s: &mut ChartStyle) -> bool {
    let mut changed = false;
    let pal = s.palette(t);
    section(ui, t, lang.text("Chart basic styles"));
    form_row(ui, t, lang.text("Background"), |ui| {
        changed |= crate::widgets::segmented(
            ui,
            t,
            &mut s.gradient,
            &[(false, lang.text("Solid colour")), (true, lang.text("Gradient"))],
        );
        ui.add_space(6.0);
        changed |= theme_color(ui, lang, t, &mut s.background, pal.bg);
        if s.gradient {
            ui.label(egui::RichText::new(crate::icons::ph::ARROW_RIGHT).color(t.text3));
            let bottom = crate::chart::style::default_bottom(t);
            changed |= theme_color(ui, lang, t, &mut s.background2, pal.bg2.unwrap_or(bottom));
        }
    });
    form_row(ui, t, lang.text("Vert grid lines"), |ui| {
        changed |= crate::widgets::checkbox(ui, &mut s.show_grid_vert, "").changed();
        changed |= theme_color(ui, lang, t, &mut s.grid_vert, pal.grid_v);
    });
    form_row(ui, t, lang.text("Horz grid lines"), |ui| {
        changed |= crate::widgets::checkbox(ui, &mut s.show_grid_horz, "").changed();
        changed |= theme_color(ui, lang, t, &mut s.grid_horz, pal.grid_h);
    });
    form_row(ui, t, lang.text("Crosshair"), |ui| {
        changed |= theme_color(ui, lang, t, &mut s.crosshair, pal.crosshair);
        crate::widgets::combo("crosshair-style")
            .width(80.0)
            .selected_text(lang.text(line_style_name(s.crosshair_style)))
            .show_ui(ui, |ui| {
                for st in [LineStyle::Solid, LineStyle::Dashed, LineStyle::Dotted] {
                    changed |=
                        crate::widgets::option_value(ui, &mut s.crosshair_style, st, lang.text(line_style_name(st)))
                            .changed();
                }
            });
    });
    changed |= crate::widgets::checkbox(ui, &mut s.watermark, lang.text("Symbol watermark")).changed();
    let _ = c32;
    changed
}

#[cfg(test)]
mod tests {
    use crate::drive::Driver;
    use crate::i18n::Language;
    use crate::settings::Cat;

    #[test]
    fn swedish_chart_settings_apply_the_selected_series_kind() {
        let mut d = Driver::new("swedish-chart-settings");
        d.app.set_language(Language::Swedish);
        d.app.open_settings(Cat::Chart);
        let candles = d.text_rect("Ljus").expect("Swedish chart type");
        d.click(candles.center());
        let area = d.text_rect("Yta").expect("Swedish area option");
        d.click(area.center());
        assert_eq!(d.app.chart.style.kind, super::SeriesKind::Area);
        assert!(d.texts().iter().any(|s| s == "Fyllning"));
    }
}

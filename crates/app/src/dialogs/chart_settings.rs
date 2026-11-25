//! Chart settings, with TradingView's tabs (Symbol, Status line, Scales and
//! lines, Canvas) and a Template menu to save, apply and set defaults.
//! Changes preview live; Cancel puts them back.

use super::color::color_button;
use super::study::line_style_name;
use super::{Done, TemplateKind, form_row, frame, section};
use crate::app::App;
use crate::chart::ChartStyle;
use crate::chart::style::{SeriesKind, c32, rgba};
use crate::i18n::{tr, trf};
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
        let mut changed = false;
        let resp = crate::widgets::button(
            ui,
            crate::widgets::Kind::Secondary,
            true,
            None,
            &trf!("Template  {}", ph::CARET_DOWN),
        );
        egui::Popup::menu(&resp).show(|ui| {
            use crate::widgets as w;
            ui.set_width(250.0);
            ui.spacing_mut().item_spacing.y = 1.0;
            if w::menu_item(ui, t, Some(ph::FLOPPY_DISK), tr("Save as"), "", false).clicked() {
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
                tr("Apply defaults"),
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
                w::menu_heading(ui, t, tr("SAVED"));
            }
            for tpl in templates {
                ui.horizontal(|ui| {
                    let label = if tpl.is_default {
                        trf!("{}  (default)", tpl.name)
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
                        if ui.small_button(ph::TRASH).on_hover_text(tr("Delete")).clicked() {
                            app.delete_template(TemplateKind::Chart, &tpl.name);
                        }
                        let tip = if tpl.is_default {
                            tr("Stop using as default")
                        } else {
                            tr("Use for new charts")
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
        changed
    }

    /// The Settings page's version: tabs as a segmented control, changes
    /// applied as they're made.
    pub fn page(&mut self, ui: &mut Ui, app: &mut App, t: &Theme) {
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
                (Tab::Symbol, tr("Symbol")),
                (Tab::Status, if narrow { tr("Status") } else { tr("Status line") }),
                (Tab::Scales, if narrow { tr("Scales") } else { tr("Scales and lines") }),
                (Tab::Canvas, tr("Canvas")),
            ],
        );
        ui.add_space(10.0);
        let mut changed = match self.tab {
            Tab::Symbol => symbol_tab(ui, t, &mut self.style, &app.chart.sym_tz),
            Tab::Status => status_tab(ui, &mut self.style),
            Tab::Scales => scales_tab(ui, t, &mut self.style),
            Tab::Canvas => canvas_tab(ui, t, &mut self.style),
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
        let mut done = Done::Keep;
        let mut changed = false;
        let closed = frame(ui, t, tr("Chart settings"), 640.0, |ui| {
            let height = 440.0;
            ui.horizontal_top(|ui| {
                ui.allocate_ui_with_layout(vec2(160.0, height), Layout::top_down(Align::Min), |ui| {
                    ui.set_min_size(vec2(160.0, height));
                    for (tab, icon, label) in [
                        (Tab::Symbol, ph::CHART_BAR, tr("Symbol")),
                        (Tab::Status, ph::LIST_DASHES, tr("Status line")),
                        (Tab::Scales, ph::RULER, tr("Scales and lines")),
                        (Tab::Canvas, ph::PAINT_BRUSH, tr("Canvas")),
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
                            Tab::Symbol => symbol_tab(ui, t, &mut self.style, &app.chart.sym_tz),
                            Tab::Status => status_tab(ui, &mut self.style),
                            Tab::Scales => scales_tab(ui, t, &mut self.style),
                            Tab::Canvas => canvas_tab(ui, t, &mut self.style),
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
    if c.is_some()
        && crate::widgets::text_link(ui, t, tr("Theme"))
            .on_hover_text(tr("Follow the app theme"))
            .clicked()
    {
        *c = None;
        changed = true;
    }
    changed
}

fn up_down(ui: &mut Ui, t: &Theme, label: &str, up: &mut Rgba, down: &mut Rgba, show: Option<&mut bool>) -> bool {
    let mut changed = false;
    form_row(ui, t, label, |ui| {
        if let Some(s) = show {
            changed |= crate::widgets::checkbox(ui, s, "").changed();
        }
        changed |= color_button(ui, t, up);
        changed |= color_button(ui, t, down);
    });
    changed
}

fn symbol_tab(ui: &mut Ui, t: &Theme, s: &mut ChartStyle, exchange: &jiff::tz::TimeZone) -> bool {
    let mut changed = false;
    section(ui, t, tr("Chart type"));
    form_row(ui, t, tr("Type"), |ui| {
        crate::widgets::combo("series-kind")
            .width(160.0)
            .selected_text(tr(s.kind.name()))
            .show_ui(ui, |ui| {
                for k in SeriesKind::ALL {
                    changed |= crate::widgets::option_value(ui, &mut s.kind, k, tr(k.name())).changed();
                }
            });
    });
    match s.kind {
        SeriesKind::Baseline => {
            form_row(ui, t, tr("Colours"), |ui| {
                changed |= color_button(ui, t, &mut s.baseline_up);
                changed |= color_button(ui, t, &mut s.baseline_down);
            });
            form_row(ui, t, tr("Base level"), |ui| {
                let mut level = s.baseline_level as i32;
                if crate::widgets::slider(ui, t, &mut level, 0..=100, "%").changed() {
                    s.baseline_level = level as _;
                    changed = true;
                }
            });
        }
        SeriesKind::HighLow => {
            form_row(ui, t, tr("Bars"), |ui| {
                changed |= color_button(ui, t, &mut s.line_color);
            });
        }
        SeriesKind::Line | SeriesKind::LineMarkers | SeriesKind::StepLine | SeriesKind::Area => {
            form_row(ui, t, tr("Line"), |ui| {
                changed |= color_button(ui, t, &mut s.line_color);
                crate::widgets::combo("line-width")
                    .width(60.0)
                    .selected_text(trf!("{}px", s.line_width))
                    .show_ui(ui, |ui| {
                        for w in 1..=4u8 {
                            changed |= crate::widgets::option_value(ui, &mut s.line_width, w, trf!("{w}px")).changed();
                        }
                    });
            });
            if s.kind == SeriesKind::Area {
                form_row(ui, t, tr("Fill"), |ui| {
                    changed |= color_button(ui, t, &mut s.area_top);
                    changed |= color_button(ui, t, &mut s.area_bottom);
                });
            }
        }
        _ => {
            changed |=
                crate::widgets::checkbox(ui, &mut s.color_on_prev_close, tr("Color bars based on previous close"))
                    .changed();
            changed |= up_down(
                ui,
                t,
                tr("Body"),
                &mut s.up_body,
                &mut s.down_body,
                Some(&mut s.show_body),
            );
            changed |= up_down(
                ui,
                t,
                tr("Borders"),
                &mut s.up_border,
                &mut s.down_border,
                Some(&mut s.show_borders),
            );
            changed |= up_down(
                ui,
                t,
                tr("Wick"),
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
                tr("Reversal amount")
            } else {
                tr("Box size")
            },
        );
        form_row(ui, t, tr("Method"), |ui| {
            let mut atr = s.box_size.is_none();
            crate::widgets::combo("box-method")
                .selected_text(if atr { tr("ATR") } else { tr("Traditional") })
                .show_ui(ui, |ui| {
                    changed |= crate::widgets::option_value(ui, &mut atr, true, tr("ATR")).changed();
                    changed |= crate::widgets::option_value(ui, &mut atr, false, tr("Traditional")).changed();
                });
            if atr && s.box_size.is_some() {
                s.box_size = None;
            } else if !atr && s.box_size.is_none() {
                s.box_size = Some(1.0);
            }
        });
        match s.box_size.as_mut() {
            None => form_row(ui, t, tr("ATR length"), |ui| {
                changed |= ui.add(egui::DragValue::new(&mut s.atr_length).range(1..=500)).changed();
            }),
            Some(v) => form_row(ui, t, tr("Size"), |ui| {
                changed |= ui
                    .add(egui::DragValue::new(v).range(1e-8..=1e9).speed(0.1).max_decimals(8))
                    .changed();
            }),
        }
        if s.kind == SeriesKind::PointFigure {
            form_row(ui, t, tr("Reversal"), |ui| {
                changed |= ui
                    .add(egui::DragValue::new(&mut s.pnf_reversal).range(1..=20))
                    .changed();
            });
        }
    }
    if s.kind == SeriesKind::LineBreak {
        section(ui, t, tr("Line break"));
        form_row(ui, t, tr("Number of lines"), |ui| {
            changed |= ui.add(egui::DragValue::new(&mut s.line_break).range(1..=10)).changed();
        });
    }
    section(ui, t, tr("Data modification"));
    form_row(ui, t, tr("Precision"), |ui| {
        let label = s.precision.map(|p| p.to_string()).unwrap_or(tr("Default").into());
        crate::widgets::combo("chart-precision")
            .selected_text(label)
            .show_ui(ui, |ui| {
                changed |= crate::widgets::option_value(ui, &mut s.precision, None, tr("Default")).changed();
                for p in 0..=8u8 {
                    changed |= crate::widgets::option_value(ui, &mut s.precision, Some(p), p.to_string()).changed();
                }
            });
    });
    form_row(ui, t, tr("Timezone"), |ui| {
        let resp = crate::widgets::button(
            ui,
            crate::widgets::Kind::Secondary,
            true,
            None,
            &format!("{}  {}", crate::timezones::choice_name(&s.timezone), ph::CARET_DOWN),
        );
        egui::Popup::menu(&resp)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .show(|ui| {
                ui.set_width(250.0);
                changed |= crate::timezones::picker(ui, t, egui::Id::new("chart-tz-search"), &mut s.timezone, exchange);
            });
    });
    changed
}

fn status_tab(ui: &mut Ui, s: &mut ChartStyle) -> bool {
    let mut changed = false;
    for (v, label) in [
        (&mut s.show_title, tr("Title")),
        (&mut s.show_ohlc, tr("Open, high, low and close values")),
        (&mut s.show_bar_change, tr("Bar change values")),
        (&mut s.show_volume, tr("Volume")),
        (&mut s.show_study_titles, tr("Indicator titles")),
        (&mut s.show_study_inputs, tr("Indicator inputs")),
        (&mut s.show_study_values, tr("Indicator values")),
    ] {
        changed |= crate::widgets::checkbox(ui, v, label).changed();
    }
    changed
}

fn scales_tab(ui: &mut Ui, t: &Theme, s: &mut ChartStyle) -> bool {
    let mut changed = false;
    section(ui, t, tr("Price scale"));
    for (v, label) in [
        (&mut s.last_price_label, tr("Symbol last price label")),
        (&mut s.last_price_line, tr("Symbol last price line")),
        (&mut s.countdown, tr("Countdown to bar close")),
        (&mut s.ext_price_label, tr("Pre/post market price label")),
        (&mut s.ext_price_line, tr("Pre/post market price line")),
        (&mut s.study_labels, tr("Indicator value labels")),
    ] {
        changed |= crate::widgets::checkbox(ui, v, label).changed();
    }
    section(ui, t, tr("Text"));
    form_row(ui, t, tr("Scales text"), |ui| {
        changed |= theme_color(ui, t, &mut s.scale_text, t.text2);
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

fn canvas_tab(ui: &mut Ui, t: &Theme, s: &mut ChartStyle) -> bool {
    let mut changed = false;
    let pal = s.palette(t);
    section(ui, t, tr("Chart basic styles"));
    form_row(ui, t, tr("Background"), |ui| {
        changed |= crate::widgets::segmented(ui, t, &mut s.gradient, &[(false, tr("Solid")), (true, tr("Gradient"))]);
        ui.add_space(6.0);
        changed |= theme_color(ui, t, &mut s.background, pal.bg);
        if s.gradient {
            ui.label(egui::RichText::new(crate::icons::ph::ARROW_RIGHT).color(t.text3));
            let bottom = crate::chart::style::default_bottom(t);
            changed |= theme_color(ui, t, &mut s.background2, pal.bg2.unwrap_or(bottom));
        }
    });
    form_row(ui, t, tr("Vert grid lines"), |ui| {
        changed |= crate::widgets::checkbox(ui, &mut s.show_grid_vert, "").changed();
        changed |= theme_color(ui, t, &mut s.grid_vert, pal.grid_v);
    });
    form_row(ui, t, tr("Horz grid lines"), |ui| {
        changed |= crate::widgets::checkbox(ui, &mut s.show_grid_horz, "").changed();
        changed |= theme_color(ui, t, &mut s.grid_horz, pal.grid_h);
    });
    form_row(ui, t, tr("Crosshair"), |ui| {
        changed |= theme_color(ui, t, &mut s.crosshair, pal.crosshair);
        crate::widgets::combo("crosshair-style")
            .width(80.0)
            .selected_text(line_style_name(s.crosshair_style))
            .show_ui(ui, |ui| {
                for st in [LineStyle::Solid, LineStyle::Dashed, LineStyle::Dotted] {
                    changed |=
                        crate::widgets::option_value(ui, &mut s.crosshair_style, st, line_style_name(st)).changed();
                }
            });
    });
    changed |= crate::widgets::checkbox(ui, &mut s.watermark, tr("Symbol watermark")).changed();
    let _ = c32;
    changed
}

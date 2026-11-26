//! Drawing settings modal, matching TradingView: Style, Text, Coordinates,
//! Inputs (for Long/Short position) and Visibility tabs, with live preview
//! on the chart and Cancel to restore original state.

use super::color::color_button;
use super::{Done, form_row, frame};
use crate::app::App;
use crate::chart::draw::{DrawStyle, Drawing, PositionParams, TextHAlign, TextVAlign, Tool, default_fib_levels};
use crate::i18n::{tr, trf};
use crate::theme::Theme;
use egui::{Align, CornerRadius, Layout, Response, Ui, vec2};
use opentern_series::study::LineStyle;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Inputs,
    Style,
    Text,
    Coordinates,
    Visibility,
}

pub struct DrawingSettings {
    pub id: u64,
    pub tab: Tab,
    pub orig: Drawing,
    pub drawing: Drawing,
    saved: bool,
}

impl DrawingSettings {
    pub fn new(app: &App, id: u64) -> Option<DrawingSettings> {
        let d = app.chart.drawings.iter().find(|d| d.id == id)?.clone();
        let tab = if matches!(d.tool, Tool::LongPosition | Tool::ShortPosition) {
            Tab::Inputs
        } else {
            Tab::Style
        };
        Some(DrawingSettings {
            id,
            tab,
            orig: d.clone(),
            drawing: d,
            saved: false,
        })
    }

    pub fn cancel(&self, app: &mut App) {
        if !self.saved
            && let Some(d) = app.chart.drawings.iter_mut().find(|d| d.id == self.id)
        {
            *d = self.orig.clone();
            app.chart.drawings_changed += 1;
        }
    }

    pub fn show(&mut self, ui: &mut Ui, app: &mut App, t: &Theme) -> Done {
        let title = tr(self.drawing.tool.name());
        let width = 460.0;
        let mut done = Done::Keep;
        let mut changed = false;

        let closed = frame(ui, t, title, width, |ui| {
            // Build available tabs for this tool.
            let mut tabs = Vec::new();
            if matches!(self.drawing.tool, Tool::LongPosition | Tool::ShortPosition) {
                tabs.push((Tab::Inputs, tr("Inputs")));
            }
            tabs.push((Tab::Style, tr("Style")));
            if supports_text(self.drawing.tool) {
                tabs.push((Tab::Text, tr("Text")));
            }
            tabs.push((Tab::Coordinates, tr("Coordinates")));
            tabs.push((Tab::Visibility, tr("Visibility")));

            // Tab switcher.
            crate::widgets::segmented(ui, t, &mut self.tab, &tabs);
            ui.add_space(8.0);

            // Tab body.
            egui::ScrollArea::vertical()
                .max_height(380.0)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 6.0;
                    match self.tab {
                        Tab::Inputs => {
                            changed |= show_inputs_tab(ui, t, &mut self.drawing, app);
                        }
                        Tab::Style => {
                            changed |= show_style_tab(ui, t, &mut self.drawing);
                        }
                        Tab::Text => {
                            changed |= show_text_tab(ui, t, &mut self.drawing);
                        }
                        Tab::Coordinates => {
                            changed |= show_coordinates_tab(ui, t, &mut self.drawing, app);
                        }
                        Tab::Visibility => {
                            changed |= show_visibility_tab(ui, t, &mut self.drawing);
                        }
                    }
                });

            ui.add_space(8.0);
            ui.separator();
            ui.add_space(6.0);

            // Bottom actions: Template menu, Cancel, OK.
            ui.horizontal(|ui| {
                crate::widgets::combo("draw-template")
                    .width(100.0)
                    .selected_text(tr("Template"))
                    .show_ui(ui, |ui| {
                        if ui
                            .button(tr("Apply defaults"))
                            .on_hover_text(tr("Reset style to defaults"))
                            .clicked()
                        {
                            self.drawing.style = DrawStyle::for_tool(self.drawing.tool);
                            changed = true;
                            ui.close();
                        }
                    });

                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.button(tr("Ok")).clicked() {
                        self.saved = true;
                        app.chart.push_undo();
                        done = Done::Close;
                    }
                    if ui.button(tr("Cancel")).clicked() {
                        self.cancel(app);
                        done = Done::Close;
                    }
                });
            });
        });

        if closed {
            self.saved = true;
            app.chart.push_undo();
            done = Done::Close;
        }

        // Live preview on chart as settings are changed.
        if changed && let Some(d) = app.chart.drawings.iter_mut().find(|d| d.id == self.id) {
            *d = self.drawing.clone();
            app.chart.drawings_changed += 1;
        }

        done
    }
}

fn supports_text(tool: Tool) -> bool {
    matches!(
        tool,
        Tool::TrendLine
            | Tool::Ray
            | Tool::ExtendedLine
            | Tool::Arrow
            | Tool::HorizontalLine
            | Tool::HorizontalRay
            | Tool::Rectangle
            | Tool::Text
            | Tool::ArrowUp
            | Tool::ArrowDown
    )
}

fn show_inputs_tab(ui: &mut Ui, t: &Theme, d: &mut Drawing, app: &App) -> bool {
    let mut changed = false;
    let params = d.style.position_params.get_or_insert_with(PositionParams::default);

    form_row(ui, t, tr("Account size"), |ui| {
        changed |= ui
            .add(
                egui::DragValue::new(&mut params.account_size)
                    .range(1.0..=1_000_000_000.0)
                    .speed(100.0),
            )
            .changed();
        ui.add(crate::widgets::field(&mut params.currency).desired_width(60.0));
    });

    form_row(ui, t, tr("Lot size"), |ui| {
        changed |= ui
            .add(
                egui::DragValue::new(&mut params.lot_size)
                    .range(0.001..=10_000.0)
                    .speed(0.1),
            )
            .changed();
    });

    form_row(ui, t, tr("Risk"), |ui| {
        changed |= ui
            .add(
                egui::DragValue::new(&mut params.risk_val)
                    .range(0.01..=1_000_000.0)
                    .speed(5.0),
            )
            .changed();
        let mode_text = if params.risk_is_pct { "%" } else { &params.currency };
        crate::widgets::combo("pos-risk-mode")
            .width(60.0)
            .selected_text(mode_text)
            .show_ui(ui, |ui| {
                if crate::widgets::option(ui, !params.risk_is_pct, &params.currency).clicked() {
                    params.risk_is_pct = false;
                    changed = true;
                }
                if crate::widgets::option(ui, params.risk_is_pct, "%").clicked() {
                    params.risk_is_pct = true;
                    changed = true;
                }
            });
    });

    if let Some(entry) = d.pts.first_mut() {
        form_row(ui, t, tr("Entry price"), |ui| {
            changed |= ui
                .add(
                    egui::DragValue::new(&mut entry.p)
                        .range(0.0..=1_000_000_000.0)
                        .speed(0.1)
                        .max_decimals(app.chart.precision()),
                )
                .changed();
        });
    }

    form_row(ui, t, tr("Leverage"), |ui| {
        changed |= ui
            .add(
                egui::DragValue::new(&mut params.leverage)
                    .range(1.0..=1000.0)
                    .speed(1.0),
            )
            .changed();
    });

    ui.add_space(4.0);
    crate::widgets::menu_heading(ui, t, &tr("PROFIT LEVEL").to_uppercase());
    if let (Some(entry), Some(target)) = (d.pts.first().copied(), d.pts.get_mut(1)) {
        let tick = app.chart.symbol.tick_size.max(1e-6);
        let mut ticks = ((target.p - entry.p).abs() / tick).round() as i64;
        form_row(ui, t, tr("Ticks"), |ui| {
            if ui
                .add(egui::DragValue::new(&mut ticks).range(1..=10_000_000).speed(1.0))
                .changed()
            {
                let sign = if d.tool == Tool::LongPosition { 1.0 } else { -1.0 };
                target.p = entry.p + sign * (ticks as f64) * tick;
                changed = true;
            }
        });
        form_row(ui, t, tr("Price"), |ui| {
            changed |= ui
                .add(
                    egui::DragValue::new(&mut target.p)
                        .range(0.0..=1_000_000_000.0)
                        .speed(0.1)
                        .max_decimals(app.chart.precision()),
                )
                .changed();
        });
    }

    ui.add_space(4.0);
    crate::widgets::menu_heading(ui, t, &tr("STOP LEVEL").to_uppercase());
    if let (Some(entry), Some(stop)) = (d.pts.first().copied(), d.pts.get_mut(2)) {
        let tick = app.chart.symbol.tick_size.max(1e-6);
        let mut ticks = ((entry.p - stop.p).abs() / tick).round() as i64;
        form_row(ui, t, tr("Ticks"), |ui| {
            if ui
                .add(egui::DragValue::new(&mut ticks).range(1..=10_000_000).speed(1.0))
                .changed()
            {
                let sign = if d.tool == Tool::LongPosition { -1.0 } else { 1.0 };
                stop.p = entry.p + sign * (ticks as f64) * tick;
                changed = true;
            }
        });
        form_row(ui, t, tr("Price"), |ui| {
            changed |= ui
                .add(
                    egui::DragValue::new(&mut stop.p)
                        .range(0.0..=1_000_000_000.0)
                        .speed(0.1)
                        .max_decimals(app.chart.precision()),
                )
                .changed();
        });
    }

    form_row(ui, t, tr("Quantity precision"), |ui| {
        let text = trf!("{} decimals", params.qty_decimals);
        crate::widgets::combo("pos-qty-prec")
            .width(110.0)
            .selected_text(text)
            .show_ui(ui, |ui| {
                for decimals in 0..=4 {
                    let label = trf!("{} decimals", decimals);
                    if crate::widgets::option(ui, params.qty_decimals == decimals, label).clicked() {
                        params.qty_decimals = decimals;
                        changed = true;
                    }
                }
            });
    });

    changed
}

fn show_style_tab(ui: &mut Ui, t: &Theme, d: &mut Drawing) -> bool {
    let mut changed = false;

    // Line / border settings.
    form_row(ui, t, tr("Line"), |ui| {
        changed |= color_button(ui, t, &mut d.style.color);
        crate::widgets::combo("draw-style-w")
            .width(60.0)
            .selected_text(trf!("{}px", d.style.width))
            .show_ui(ui, |ui| {
                for w in 1..=4u8 {
                    changed |= crate::widgets::option_value(ui, &mut d.style.width, w, trf!("{w}px")).changed();
                }
            });
        let line_name = |s: LineStyle| match s {
            LineStyle::Solid => tr("Solid"),
            LineStyle::Dashed => tr("Dashed"),
            LineStyle::Dotted => tr("Dotted"),
        };
        crate::widgets::combo("draw-style-l")
            .width(80.0)
            .selected_text(line_name(d.style.line))
            .show_ui(ui, |ui| {
                for s in [LineStyle::Solid, LineStyle::Dashed, LineStyle::Dotted] {
                    changed |= crate::widgets::option_value(ui, &mut d.style.line, s, line_name(s)).changed();
                }
            });
    });

    // Fills for shapes.
    let has_fill = matches!(
        d.tool,
        Tool::Rectangle
            | Tool::Ellipse
            | Tool::Triangle
            | Tool::ParallelChannel
            | Tool::PriceRange
            | Tool::DateRange
            | Tool::DatePriceRange
    );
    if has_fill {
        form_row(ui, t, tr("Background"), |ui| {
            changed |= color_button(ui, t, &mut d.style.fill);
        });
    }

    // Extend options for Lines and Rectangles.
    if matches!(d.tool, Tool::TrendLine | Tool::Arrow) {
        form_row(ui, t, tr("Extend"), |ui| {
            changed |= crate::widgets::checkbox(ui, &mut d.style.extend_left, tr("Extend left")).changed();
            changed |= crate::widgets::checkbox(ui, &mut d.style.extend_right, tr("Extend right")).changed();
        });
    } else if d.tool == Tool::Rectangle {
        form_row(ui, t, tr("Extend"), |ui| {
            let label = match (d.style.extend_left, d.style.extend_right) {
                (false, false) => tr("Don't extend"),
                (false, true) => tr("Extend right"),
                (true, false) => tr("Extend left"),
                (true, true) => tr("Extend both"),
            };
            crate::widgets::combo("rect-extend")
                .width(130.0)
                .selected_text(label)
                .show_ui(ui, |ui| {
                    if crate::widgets::option(ui, !d.style.extend_left && !d.style.extend_right, tr("Don't extend"))
                        .clicked()
                    {
                        d.style.extend_left = false;
                        d.style.extend_right = false;
                        changed = true;
                    }
                    if crate::widgets::option(ui, !d.style.extend_left && d.style.extend_right, tr("Extend right"))
                        .clicked()
                    {
                        d.style.extend_left = false;
                        d.style.extend_right = true;
                        changed = true;
                    }
                    if crate::widgets::option(ui, d.style.extend_left && !d.style.extend_right, tr("Extend left"))
                        .clicked()
                    {
                        d.style.extend_left = true;
                        d.style.extend_right = false;
                        changed = true;
                    }
                    if crate::widgets::option(ui, d.style.extend_left && d.style.extend_right, tr("Extend both"))
                        .clicked()
                    {
                        d.style.extend_left = true;
                        d.style.extend_right = true;
                        changed = true;
                    }
                });
        });

        // Middle line toggle for Rectangle.
        form_row(ui, t, tr("Middle line"), |ui| {
            changed |= crate::widgets::checkbox(ui, &mut d.style.middle_line, "").changed();
            if d.style.middle_line {
                let mut ml_color = d.style.middle_line_color.unwrap_or(d.style.color);
                if color_button(ui, t, &mut ml_color) {
                    d.style.middle_line_color = Some(ml_color);
                    changed = true;
                }
            }
        });
    }

    // Fib Retracement style settings.
    if matches!(d.tool, Tool::FibRetracement | Tool::FibExtension) {
        form_row(ui, t, tr("Trend line"), |ui| {
            changed |= crate::widgets::checkbox(ui, &mut d.style.show_trendline, "").changed();
            if d.style.show_trendline {
                let mut tc = d.style.trendline_color.unwrap_or(d.style.color);
                if color_button(ui, t, &mut tc) {
                    d.style.trendline_color = Some(tc);
                    changed = true;
                }
            }
        });

        form_row(ui, t, tr("Extend lines"), |ui| {
            let label = match (d.style.extend_left, d.style.extend_right) {
                (false, false) => tr("Don't extend"),
                (false, true) => tr("Extend lines right"),
                (true, false) => tr("Extend lines left"),
                (true, true) => tr("Extend lines"),
            };
            crate::widgets::combo("fib-extend")
                .width(140.0)
                .selected_text(label)
                .show_ui(ui, |ui| {
                    if crate::widgets::option(ui, !d.style.extend_left && !d.style.extend_right, tr("Don't extend"))
                        .clicked()
                    {
                        d.style.extend_left = false;
                        d.style.extend_right = false;
                        changed = true;
                    }
                    if crate::widgets::option(
                        ui,
                        !d.style.extend_left && d.style.extend_right,
                        tr("Extend lines right"),
                    )
                    .clicked()
                    {
                        d.style.extend_left = false;
                        d.style.extend_right = true;
                        changed = true;
                    }
                    if crate::widgets::option(
                        ui,
                        d.style.extend_left && !d.style.extend_right,
                        tr("Extend lines left"),
                    )
                    .clicked()
                    {
                        d.style.extend_left = true;
                        d.style.extend_right = false;
                        changed = true;
                    }
                    if crate::widgets::option(ui, d.style.extend_left && d.style.extend_right, tr("Extend lines"))
                        .clicked()
                    {
                        d.style.extend_left = true;
                        d.style.extend_right = true;
                        changed = true;
                    }
                });
        });

        form_row(ui, t, tr("Background"), |ui| {
            changed |= crate::widgets::checkbox(ui, &mut d.style.fib_background, "").changed();
            if d.style.fib_background {
                let mut op = d.style.fib_opacity as i32;
                if crate::widgets::slider(ui, t, &mut op, 0..=100, "%").changed() {
                    d.style.fib_opacity = op.clamp(0, 100) as u8;
                    changed = true;
                }
            }
        });

        form_row(ui, t, tr("Price labels"), |ui| {
            changed |= crate::widgets::checkbox(ui, &mut d.style.labels, "").changed();
        });

        ui.add_space(4.0);
        crate::widgets::menu_heading(ui, t, &tr("LEVELS").to_uppercase());

        let levels = d.style.fib_levels.get_or_insert_with(default_fib_levels);
        let half = levels.len().div_ceil(2);
        ui.columns(2, |cols| {
            for (col_idx, ui) in cols.iter_mut().enumerate() {
                let start = col_idx * half;
                let end = ((col_idx + 1) * half).min(levels.len());
                for lvl in &mut levels[start..end] {
                    ui.horizontal(|ui| {
                        changed |= crate::widgets::checkbox(ui, &mut lvl.enabled, "").changed();
                        changed |= ui
                            .add(
                                egui::DragValue::new(&mut lvl.level)
                                    .range(-100.0..=100.0)
                                    .speed(0.01)
                                    .max_decimals(3),
                            )
                            .changed();
                        changed |= color_button(ui, t, &mut lvl.color);
                    });
                }
            }
        });
    }

    // Long / Short position style settings.
    if matches!(d.tool, Tool::LongPosition | Tool::ShortPosition) {
        let params = d.style.position_params.get_or_insert_with(PositionParams::default);

        form_row(ui, t, tr("Target color"), |ui| {
            let mut tc = params.target_color.unwrap_or(opentern_series::Rgba::hex(0x089981));
            if color_button(ui, t, &mut tc) {
                params.target_color = Some(tc);
                changed = true;
            }
        });

        form_row(ui, t, tr("Stop color"), |ui| {
            let mut sc = params.stop_color.unwrap_or(opentern_series::Rgba::hex(0xF23645));
            if color_button(ui, t, &mut sc) {
                params.stop_color = Some(sc);
                changed = true;
            }
        });

        form_row(ui, t, tr("Text"), |ui| {
            let mut tc = params.text_color.unwrap_or(opentern_series::Rgba::hex(0xFFFFFF));
            if color_button(ui, t, &mut tc) {
                params.text_color = Some(tc);
                changed = true;
            }
            crate::widgets::combo("pos-text-size")
                .width(60.0)
                .selected_text(format!("{}px", params.text_size))
                .show_ui(ui, |ui| {
                    for s in [9u8, 10, 11, 12, 14, 16] {
                        if crate::widgets::option(ui, params.text_size == s, format!("{s}px")).clicked() {
                            params.text_size = s;
                            changed = true;
                        }
                    }
                });
        });

        form_row(ui, t, tr("Price labels"), |ui| {
            changed |= crate::widgets::checkbox(ui, &mut params.show_price_labels, "").changed();
        });

        form_row(ui, t, tr("Compact stats"), |ui| {
            changed |= crate::widgets::checkbox(ui, &mut params.compact_stats, "").changed();
        });

        form_row(ui, t, tr("Always show stats"), |ui| {
            changed |= crate::widgets::checkbox(ui, &mut params.always_show_stats, "").changed();
        });
    }

    changed
}

fn show_text_tab(ui: &mut Ui, t: &Theme, d: &mut Drawing) -> bool {
    let mut changed = false;

    form_row(ui, t, tr("Text style"), |ui| {
        let mut tc = d.style.text_color.unwrap_or(d.style.color);
        if color_button(ui, t, &mut tc) {
            d.style.text_color = Some(tc);
            changed = true;
        }

        let size = if d.style.text_size > 0 { d.style.text_size } else { 12 };
        crate::widgets::combo("draw-text-size")
            .width(60.0)
            .selected_text(format!("{size}"))
            .show_ui(ui, |ui| {
                for s in [10u8, 11, 12, 14, 16, 20, 24, 28, 36] {
                    if crate::widgets::option(ui, size == s, format!("{s}")).clicked() {
                        d.style.text_size = s;
                        changed = true;
                    }
                }
            });

        changed |= toggle_btn(ui, t, "B", &mut d.style.text_bold).changed();
        changed |= toggle_btn(ui, t, "I", &mut d.style.text_italic).changed();
    });

    if matches!(d.tool, Tool::Rectangle | Tool::HorizontalLine | Tool::HorizontalRay) {
        form_row(ui, t, tr("Text alignment"), |ui| {
            let vname = match d.style.text_valign {
                TextVAlign::Top => tr("Top"),
                TextVAlign::Inside => tr("Inside"),
                TextVAlign::Bottom => tr("Bottom"),
            };
            crate::widgets::combo("rect-valign")
                .width(80.0)
                .selected_text(vname)
                .show_ui(ui, |ui| {
                    if crate::widgets::option(ui, d.style.text_valign == TextVAlign::Top, tr("Top")).clicked() {
                        d.style.text_valign = TextVAlign::Top;
                        changed = true;
                    }
                    if crate::widgets::option(ui, d.style.text_valign == TextVAlign::Inside, tr("Inside")).clicked() {
                        d.style.text_valign = TextVAlign::Inside;
                        changed = true;
                    }
                    if crate::widgets::option(ui, d.style.text_valign == TextVAlign::Bottom, tr("Bottom")).clicked() {
                        d.style.text_valign = TextVAlign::Bottom;
                        changed = true;
                    }
                });

            let hname = match d.style.text_halign {
                TextHAlign::Left => tr("Left"),
                TextHAlign::Center => tr("Center"),
                TextHAlign::Right => tr("Right"),
            };
            crate::widgets::combo("rect-halign")
                .width(80.0)
                .selected_text(hname)
                .show_ui(ui, |ui| {
                    if crate::widgets::option(ui, d.style.text_halign == TextHAlign::Left, tr("Left")).clicked() {
                        d.style.text_halign = TextHAlign::Left;
                        changed = true;
                    }
                    if crate::widgets::option(ui, d.style.text_halign == TextHAlign::Center, tr("Center")).clicked() {
                        d.style.text_halign = TextHAlign::Center;
                        changed = true;
                    }
                    if crate::widgets::option(ui, d.style.text_halign == TextHAlign::Right, tr("Right")).clicked() {
                        d.style.text_halign = TextHAlign::Right;
                        changed = true;
                    }
                });
        });
    }

    ui.add_space(6.0);
    form_row(ui, t, tr("Text"), |ui| {
        changed |= ui
            .add(
                egui::TextEdit::multiline(&mut d.text)
                    .desired_width(280.0)
                    .desired_rows(4)
                    .hint_text(tr("Add text")),
            )
            .changed();
    });

    changed
}

fn show_coordinates_tab(ui: &mut Ui, t: &Theme, d: &mut Drawing, app: &App) -> bool {
    let mut changed = false;
    let decimals = app.chart.precision();

    for (i, pt) in d.pts.iter_mut().enumerate() {
        let label = format!("#{} ({})", i + 1, tr("price, bar"));
        form_row(ui, t, &label, |ui| {
            changed |= ui
                .add(
                    egui::DragValue::new(&mut pt.p)
                        .range(0.0..=1_000_000_000.0)
                        .speed(0.1)
                        .max_decimals(decimals),
                )
                .changed();

            let step = app.chart.tf.millis().unwrap_or(86_400_000);
            let mut bar_idx = bar_index_at_time(&app.chart.bars, pt.t, step);
            if ui.add(egui::DragValue::new(&mut bar_idx).speed(1.0)).changed() {
                pt.t = time_at_index(&app.chart.bars, bar_idx, step);
                changed = true;
            }
        });
    }

    changed
}

fn show_visibility_tab(ui: &mut Ui, t: &Theme, d: &mut Drawing) -> bool {
    let mut changed = false;
    form_row(ui, t, tr("Visibility"), |ui| {
        let mut visible = !d.hidden;
        if crate::widgets::checkbox(ui, &mut visible, tr("Show on chart")).changed() {
            d.hidden = !visible;
            changed = true;
        }
    });
    changed
}

fn toggle_btn(ui: &mut Ui, t: &Theme, text: &str, active: &mut bool) -> Response {
    let size = vec2(26.0, 26.0);
    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
    if resp.clicked() {
        *active = !*active;
    }
    if ui.is_rect_visible(rect) {
        let p = ui.painter();
        let bg = if *active {
            t.accent_soft()
        } else if resp.hovered() {
            t.hover_wash()
        } else {
            egui::Color32::TRANSPARENT
        };
        p.rect_filled(rect, CornerRadius::same(5), bg);
        let fg = if *active { t.accent } else { t.text };
        p.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            text,
            crate::theme::semibold(12.0),
            fg,
        );
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn time_at_index(bars: &[opentern_core::Bar], i: i64, step: i64) -> i64 {
    if bars.is_empty() {
        return 0;
    }
    let last = (bars.len() - 1) as i64;
    if i < 0 {
        bars[0].time + i * step
    } else if i > last {
        bars[bars.len() - 1].time + (i - last) * step
    } else {
        bars[i as usize].time
    }
}

fn bar_index_at_time(bars: &[opentern_core::Bar], t: i64, step: i64) -> i64 {
    if bars.is_empty() {
        return 0;
    }
    if let Ok(idx) = bars.binary_search_by_key(&t, |b| b.time) {
        return idx as i64;
    }
    let last = (bars.len() - 1) as i64;
    if t < bars[0].time {
        let diff = (bars[0].time - t) as f64 / step.max(1) as f64;
        -(diff.round() as i64)
    } else if t > bars[bars.len() - 1].time {
        let diff = (t - bars[bars.len() - 1].time) as f64 / step.max(1) as f64;
        last + diff.round() as i64
    } else {
        bars.partition_point(|b| b.time < t) as i64
    }
}

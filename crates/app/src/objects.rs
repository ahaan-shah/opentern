//! The Object tree (everything on the chart: indicators and drawings, with
//! show/hide, lock and remove) and the Data window (values under the
//! crosshair).

use crate::app::App;
use crate::format;
use crate::i18n::{tr, trc};
use crate::icons::ph;
use crate::theme::{self, Theme};
use egui::{CornerRadius, Sense, Ui, pos2, vec2};
use opentern_series::study::Format;

impl App {
    pub(crate) fn objects_widget(&mut self, ui: &mut Ui, t: &Theme) {
        egui::ScrollArea::vertical().id_salt("objects").show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 2.0;
            heading(ui, t, tr("Object tree"));
            ui.add(
                egui::Label::new(
                    egui::RichText::new(format!("{} · {}", self.chart.symbol.ticker(), self.chart.tf.label()))
                        .font(theme::medium(13.0))
                        .color(t.text2),
                )
                .truncate(),
            );
            ui.add_space(4.0);
            if self.chart.studies.is_empty() && self.chart.drawings.is_empty() {
                crate::widgets::empty_state(ui, t, ph::STACK, tr("Nothing on the chart yet"));
            }
            let mut remove_study = None;
            let mut settings = None;
            // Titles name a linked source's study, so they need the chart.
            let titles: Vec<String> = self
                .chart
                .studies
                .iter()
                .map(|s| self.chart.study_title(s, true))
                .collect();
            for (k, s) in self.chart.studies.iter_mut().enumerate() {
                let eye = if s.visible { ph::EYE } else { ph::EYE_SLASH };
                let pick = object_row(
                    ui,
                    t,
                    ("study", s.uid),
                    ph::FUNCTION,
                    &titles[k],
                    false,
                    !s.visible,
                    [
                        (eye, tr("Show or hide"), !s.visible),
                        (ph::GEAR_SIX, tr("Settings"), false),
                        (ph::X, tr("Remove"), false),
                    ],
                );
                match pick {
                    Some(Pick::Button(0)) => s.visible = !s.visible,
                    Some(Pick::Button(1)) => settings = Some(s.uid),
                    Some(Pick::Button(_)) => remove_study = Some(s.uid),
                    Some(Pick::Row) | None => {}
                }
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
                let eye = if d.hidden { ph::EYE_SLASH } else { ph::EYE };
                let lock = if d.locked {
                    ph::LOCK_SIMPLE
                } else {
                    ph::LOCK_SIMPLE_OPEN
                };
                // Its text names it best, when it has some.
                let name = if d.text.is_empty() {
                    tr(d.tool.name()).to_string()
                } else {
                    format!("{} · {}", tr(d.tool.name()), d.text)
                };
                let on = self.chart_ui.draw.is_selected(d.id);
                match object_row(
                    ui,
                    t,
                    ("drawing", d.id),
                    d.tool.icon(),
                    &name,
                    on,
                    d.hidden,
                    [
                        (eye, tr("Show or hide"), d.hidden),
                        (lock, tr("Lock"), d.locked),
                        (ph::X, tr("Remove"), false),
                    ],
                ) {
                    Some(Pick::Button(0)) => {
                        d.hidden = !d.hidden;
                        changed = true;
                    }
                    Some(Pick::Button(1)) => {
                        d.locked = !d.locked;
                        changed = true;
                    }
                    Some(Pick::Button(_)) => remove = Some(d.id),
                    Some(Pick::Row) => select = Some(d.id),
                    None => {}
                }
            }
            if changed {
                self.chart.drawings_changed += 1;
            }
            if let Some(id) = select {
                self.chart_ui.draw.select(Some(id));
            }
            if let Some(id) = remove {
                self.chart.push_undo();
                self.chart.drawings.retain(|d| d.id != id);
                self.chart.drawings_changed += 1;
            }
            ui.add_space(14.0);
            heading(ui, t, tr("Data window"));
            self.data_window(ui, t);
        });
    }

    fn data_window(&self, ui: &mut Ui, t: &Theme) {
        let c = &self.chart;
        let Some(i) = self.chart_ui.hover_index.filter(|i| *i < c.bars.len()) else {
            ui.horizontal(|ui| {
                ui.add_space(4.0);
                ui.label(egui::RichText::new(tr("Move the pointer over the chart.")).color(t.text3));
            });
            return;
        };
        let b = c.bars[i];
        let prec = c.precision();
        // The name at the left, the value at the right, in one quiet row.
        let row = |ui: &mut Ui, k: &str, v: String| {
            let (r, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 24.0), egui::Sense::hover());
            let p = ui.painter();
            let font = theme::regular(12.5);
            let value = p.text(
                r.right_center() - egui::vec2(4.0, 0.0),
                egui::Align2::RIGHT_CENTER,
                v,
                font.clone(),
                t.text,
            );
            let name = format::elide_to(p, k, &font, value.left() - r.left() - 12.0);
            let shortened = name != k;
            p.text(
                r.left_center() + egui::vec2(4.0, 0.0),
                egui::Align2::LEFT_CENTER,
                name,
                font,
                t.text2,
            );
            if shortened {
                resp.on_hover_text(k);
            }
        };
        row(ui, tr("Date"), format::crosshair_time(b.time, c.tf, &c.tz));
        row(ui, trc("price", "Open"), format::price(b.open, prec));
        row(ui, trc("price", "High"), format::price(b.high, prec));
        row(ui, trc("price", "Low"), format::price(b.low, prec));
        row(ui, trc("price", "Close"), format::price(b.close, prec));
        if i > 0 {
            let p = c.bars[i - 1].close;
            row(
                ui,
                tr("Change"),
                format!(
                    "{} ({})",
                    format::price(b.close - p, prec),
                    format::signed_pct((b.close / p - 1.0) * 100.0)
                ),
            );
        }
        row(ui, tr("Volume"), format::compact_for(b.volume, &c.symbol));
        for s in &c.studies {
            ui.add_space(8.0);
            let title = c.study_title(s, true);
            ui.horizontal(|ui| {
                ui.add_space(4.0);
                ui.add(egui::Label::new(egui::RichText::new(&title).font(theme::medium(12.5))).truncate())
                    .on_hover_text(&title);
            });
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
                    Format::Volume => format::compact_for(v, &c.symbol),
                };
                row(ui, crate::i18n::of_study(s.meta(), spec.title), text);
            }
        }
    }
}

/// A section's name, in plain case, as the Script Editor's and the
/// calendar's.
fn heading(ui: &mut Ui, t: &Theme, text: &str) {
    ui.add_space(2.0);
    ui.label(egui::RichText::new(text).font(theme::medium(12.0)).color(t.text3));
    ui.add_space(2.0);
}

/// What was clicked on an object row.
enum Pick {
    Row,
    Button(usize),
}

/// One object on the chart: a rounded row as the watchlist's, its icon and
/// name, and three icon buttons at the right. The buttons show on hover;
/// one that's on (hidden, locked) stays shown. A hidden object's row is dim.
#[allow(clippy::too_many_arguments)]
fn object_row(
    ui: &mut Ui,
    t: &Theme,
    id: impl std::hash::Hash + std::fmt::Debug,
    icon: &str,
    name: &str,
    selected: bool,
    dim: bool,
    buttons: [(&str, &str, bool); 3],
) -> Option<Pick> {
    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 34.0), Sense::click());
    let inside = ui.rect_contains_pointer(r);
    let hover = crate::motion::hover_if(&resp, inside);
    let p = ui.painter();
    if selected {
        p.rect_filled(r, CornerRadius::same(7), t.accent_soft());
    } else {
        p.rect_filled(
            r,
            CornerRadius::same(7),
            crate::motion::with_alpha(t.hover_wash(), hover),
        );
    }
    let fg = match (selected, dim) {
        (true, _) => t.accent,
        (_, true) => t.text3,
        _ => t.text,
    };
    p.text(
        pos2(r.left() + 16.0, r.center().y),
        egui::Align2::CENTER_CENTER,
        icon,
        theme::regular(14.0),
        if selected { t.accent } else { t.text3 },
    );
    let mut pick = None;
    let mut x = r.right() - 4.0;
    for (k, (glyph, tip, on)) in buttons.iter().enumerate().rev() {
        let br = egui::Rect::from_center_size(pos2(x - 13.0, r.center().y), vec2(26.0, 26.0));
        x -= 28.0;
        let shown = if *on { 1.0 } else { hover };
        if shown <= 0.0 {
            continue;
        }
        let b = ui.interact(br, ui.id().with((&id, k)), Sense::click());
        let col = crate::motion::with_alpha(
            crate::motion::lerp_color(t.text2, t.text, crate::motion::hover(&b)),
            shown,
        );
        ui.painter().text(
            br.center(),
            egui::Align2::CENTER_CENTER,
            *glyph,
            theme::regular(14.0),
            col,
        );
        if b.on_hover_text(*tip)
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .clicked()
        {
            pick = Some(Pick::Button(k));
        }
    }
    let room = if inside || buttons.iter().any(|b| b.2) {
        x
    } else {
        r.right() - 8.0
    };
    let font = theme::medium(13.0);
    let shown = format::elide_to(ui.painter(), name, &font, room - (r.left() + 34.0) - 6.0);
    let cut = shown != name;
    ui.painter().text(
        pos2(r.left() + 34.0, r.center().y),
        egui::Align2::LEFT_CENTER,
        shown,
        font,
        fg,
    );
    if pick.is_none() && resp.clicked() {
        pick = Some(Pick::Row);
    }
    if cut {
        resp.on_hover_text(name);
    }
    pick
}

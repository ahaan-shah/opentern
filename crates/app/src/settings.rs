//! Settings: a page that takes the chart area (☰ or the gear opens it,
//! Esc or × closes it), with its categories down the left: Appearance,
//! Chart, Data providers and Symbol logos.

use crate::app::App;
use crate::dialogs::ChartSettings;
use crate::dialogs::providers::Providers;
use crate::icons::ph;
use crate::motion;
use crate::theme::{self, THEMES, Theme};
use egui::{Align2, CornerRadius, Rect, Sense, Stroke, Ui, pos2, vec2};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Cat {
    Appearance,
    Chart,
    Providers,
    Logos,
}

impl Cat {
    pub const ALL: [Cat; 4] = [Cat::Appearance, Cat::Chart, Cat::Providers, Cat::Logos];

    fn title(self) -> &'static str {
        match self {
            Cat::Appearance => "Appearance",
            Cat::Chart => "Chart",
            Cat::Providers => "Data providers",
            Cat::Logos => "Symbol logos",
        }
    }

    fn icon(self) -> &'static str {
        match self {
            Cat::Appearance => ph::PALETTE,
            Cat::Chart => ph::CHART_LINE,
            Cat::Providers => ph::PLUGS_CONNECTED,
            Cat::Logos => ph::IMAGE,
        }
    }

    fn blurb(self) -> &'static str {
        match self {
            Cat::Appearance => "The app's colours. Charts follow them until you give a chart colours of its own.",
            Cat::Chart => "The active chart's look. Save it as a template to use it on other charts.",
            Cat::Providers => "Sources that need an account. Everything else works without one.",
            Cat::Logos => "Round logos beside symbols, as on TradingView.",
        }
    }
}

pub struct SettingsPage {
    pub cat: Cat,
    providers: Option<Providers>,
    chart: Option<ChartSettings>,
}

impl SettingsPage {
    pub fn new(cat: Cat) -> SettingsPage {
        SettingsPage {
            cat,
            providers: None,
            chart: None,
        }
    }
}

impl App {
    /// Opens Settings at `cat` (or closes it if it's already showing that).
    pub(crate) fn open_settings(&mut self, cat: Cat) {
        match &mut self.settings {
            Some(s) if s.cat == cat => self.settings = None,
            Some(s) => s.cat = cat,
            None => self.settings = Some(SettingsPage::new(cat)),
        }
    }

    pub(crate) fn settings_page(&mut self, ui: &mut Ui, t: &Theme) {
        let Some(mut page) = self.settings.take() else { return };
        let full = ui.max_rect();
        ui.painter().rect_filled(full, CornerRadius::ZERO, t.bg);
        let nav_w = 220.0;
        let nav = Rect::from_min_size(full.min, vec2(nav_w, full.height()));
        ui.painter().vline(
            nav.right(),
            full.y_range(),
            Stroke::new(1.0, t.border.gamma_multiply(0.7)),
        );
        let mut close = false;

        // Categories.
        let mut c = ui.new_child(egui::UiBuilder::new().max_rect(nav.shrink2(vec2(12.0, 16.0))));
        c.label(
            egui::RichText::new("Settings")
                .font(theme::semibold(18.0))
                .color(t.text),
        );
        c.add_space(14.0);
        for cat in Cat::ALL {
            let (r, resp) = c.allocate_exact_size(vec2(c.available_width(), 34.0), Sense::click());
            let on = page.cat == cat;
            let hov = motion::toggle(c.ctx(), resp.id, resp.hovered(), motion::MICRO);
            let p = c.painter();
            p.rect_filled(r, CornerRadius::same(8), motion::with_alpha(t.hover_wash(), hov));
            if on {
                p.rect_filled(r, CornerRadius::same(8), t.accent_soft());
            }
            let col = if on { t.accent } else { t.text };
            p.text(
                pos2(r.left() + 12.0, r.center().y),
                Align2::LEFT_CENTER,
                cat.icon(),
                theme::regular(15.0),
                col,
            );
            p.text(
                pos2(r.left() + 36.0, r.center().y),
                Align2::LEFT_CENTER,
                cat.title(),
                theme::medium(13.5),
                col,
            );
            if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                page.cat = cat;
            }
            c.add_space(2.0);
        }

        // The category.
        let body = Rect::from_min_max(pos2(nav.right() + 1.0, full.top()), full.max);
        let close_r = Rect::from_min_size(pos2(body.right() - 44.0, body.top() + 12.0), vec2(30.0, 30.0));
        let mut c = ui.new_child(egui::UiBuilder::new().max_rect(close_r));
        if crate::widgets::tool_button(&mut c, t, ph::X, "Close (Esc)", false, 30.0).clicked() {
            close = true;
        }
        let inner = Rect::from_min_max(
            pos2(body.left() + 32.0, body.top() + 18.0),
            pos2(body.right() - 24.0, body.bottom() - 8.0),
        );
        let mut c = ui.new_child(egui::UiBuilder::new().max_rect(inner));
        c.set_clip_rect(inner.intersect(ui.clip_rect()));
        c.label(
            egui::RichText::new(page.cat.title())
                .font(theme::semibold(18.0))
                .color(t.text),
        );
        c.add_space(2.0);
        c.label(
            egui::RichText::new(page.cat.blurb())
                .font(theme::regular(12.5))
                .color(t.text3),
        );
        c.add_space(14.0);
        egui::ScrollArea::vertical()
            .id_salt(("settings", page.cat as u8))
            .auto_shrink([false; 2])
            .show(&mut c, |ui| {
                let w = (ui.available_width() - 14.0).min(720.0);
                ui.set_max_width(w);
                match page.cat {
                    Cat::Appearance => self.appearance(ui, t),
                    Cat::Chart => page
                        .chart
                        .get_or_insert_with(|| ChartSettings::new(self))
                        .page(ui, self, t),
                    Cat::Providers => page
                        .providers
                        .get_or_insert_with(|| self.providers_form())
                        .page(ui, self, t),
                    Cat::Logos => self.logo_settings(ui, t),
                }
                ui.add_space(24.0);
            });
        if !close && self.settings.is_none() {
            self.settings = Some(page);
        }
    }

    fn appearance(&mut self, ui: &mut Ui, t: &Theme) {
        crate::dialogs::section(ui, t, "Theme");
        ui.add_space(4.0);
        let current = self.t().name;
        let mut pick = None;
        // A card per theme: its colours, its name, a check on the one in use.
        // As many as fit, sharing the row's width (less room for the
        // scrollbar's gutter).
        let avail = ui.available_width() - 14.0;
        let per_row = ((avail + 10.0) / 160.0).floor().clamp(1.0, 4.0) as usize;
        let card = vec2((avail - 10.0 * (per_row - 1) as f32) / per_row as f32, 92.0);
        for row in THEMES.chunks(per_row) {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                for th in row {
                    let (r, resp) = ui.allocate_exact_size(card, Sense::click());
                    let on = th.name == current;
                    let hov = motion::toggle(ui.ctx(), resp.id, resp.hovered(), motion::MICRO);
                    let p = ui.painter();
                    p.rect_filled(r, CornerRadius::same(10), th.bg);
                    // A little chart: sidebar strip, a card, up and down bars.
                    let side = Rect::from_min_size(r.min, vec2(26.0, r.height()));
                    p.rect_filled(
                        side,
                        CornerRadius {
                            nw: 10,
                            sw: 10,
                            ne: 0,
                            se: 0,
                        },
                        th.sidebar,
                    );
                    let cardr = Rect::from_min_max(
                        pos2(r.left() + 34.0, r.top() + 10.0),
                        pos2(r.right() - 8.0, r.top() + 58.0),
                    );
                    p.rect_filled(cardr, CornerRadius::same(6), th.card);
                    for (k, (h, up)) in [(14.0, true), (22.0, false), (18.0, true), (30.0, true), (12.0, false)]
                        .into_iter()
                        .enumerate()
                    {
                        let x = cardr.left() + 12.0 + k as f32 * 16.0;
                        let col = if up { th.pos } else { th.neg };
                        p.rect_filled(
                            Rect::from_min_max(pos2(x, cardr.bottom() - 8.0 - h), pos2(x + 8.0, cardr.bottom() - 8.0)),
                            CornerRadius::same(2),
                            col,
                        );
                    }
                    p.circle_filled(pos2(side.center().x, r.top() + 16.0), 5.0, th.accent);
                    p.text(
                        pos2(r.left() + 34.0, r.bottom() - 16.0),
                        Align2::LEFT_CENTER,
                        th.name,
                        theme::medium(12.0),
                        th.text,
                    );
                    if on {
                        p.text(
                            pos2(r.right() - 12.0, r.bottom() - 16.0),
                            Align2::CENTER_CENTER,
                            ph::CHECK_CIRCLE,
                            theme::regular(15.0),
                            th.accent,
                        );
                    }
                    let edge = if on {
                        Stroke::new(2.0, t.accent)
                    } else {
                        Stroke::new(1.0, motion::lerp_color(t.border, t.text3, hov))
                    };
                    p.rect_stroke(r, CornerRadius::same(10), edge, egui::StrokeKind::Inside);
                    if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                        pick = Some(th.name);
                    }
                }
            });
            ui.add_space(10.0);
        }
        if let Some(name) = pick {
            self.set_theme(ui.ctx(), name);
        }
    }

    fn logo_settings(&mut self, ui: &mut Ui, t: &Theme) {
        let mut on = self.logos.enabled;
        if crate::widgets::checkbox(ui, &mut on, "Show symbol logos").changed() {
            self.logos.enabled = on;
            let _ = self.store.set_value("symbol_logos", if on { "on" } else { "off" });
        }
        ui.add_space(8.0);
        let note = |ui: &mut Ui, s: &str| {
            ui.label(egui::RichText::new(s).font(theme::regular(12.5)).color(t.text2));
        };
        note(
            ui,
            "Companies' marks come from theSVG (brand SVGs), drawn white on the brand's colour; coins from the \
             cryptocurrency-icons set. Anything without one gets its first letter.",
        );
        note(
            ui,
            "Each logo is fetched once and kept on this computer. Nothing else is sent: only the logo's name.",
        );
        ui.add_space(10.0);
        if crate::widgets::secondary(ui, "Clear saved logos").clicked() {
            self.logos.clear();
        }
    }
}

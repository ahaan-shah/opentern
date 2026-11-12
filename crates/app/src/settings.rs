//! Settings: a page that takes the chart area (☰ or the gear opens it,
//! Esc or × closes it), with its categories down the left: General
//! (folders, theme, symbol logos), Chart, Data providers, Keyboard
//! shortcuts, Export and import, and About.

use crate::app::App;
use crate::dialogs::ChartSettings;
use crate::dialogs::providers::Providers;
use crate::icons::ph;
use crate::motion;
use crate::theme::{self, THEMES, Theme};
use egui::{Align2, CornerRadius, Rect, Sense, Stroke, Ui, pos2, vec2};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Cat {
    General,
    Chart,
    Providers,
    Shortcuts,
    Transfer,
    About,
}

impl Cat {
    pub const ALL: [Cat; 6] = [
        Cat::General,
        Cat::Chart,
        Cat::Providers,
        Cat::Shortcuts,
        Cat::Transfer,
        Cat::About,
    ];

    fn title(self) -> &'static str {
        match self {
            Cat::General => "General",
            Cat::Chart => "Chart",
            Cat::Providers => "Data providers",
            Cat::Shortcuts => "Keyboard shortcuts",
            Cat::Transfer => "Export and import",
            Cat::About => "About",
        }
    }

    fn icon(self) -> &'static str {
        match self {
            Cat::General => ph::SLIDERS_HORIZONTAL,
            Cat::Chart => ph::CHART_LINE,
            Cat::Providers => ph::PLUGS_CONNECTED,
            Cat::Shortcuts => ph::KEYBOARD,
            Cat::Transfer => ph::ARROWS_LEFT_RIGHT,
            Cat::About => ph::INFO,
        }
    }
}

/// A folder Settings → General can move out of the workspace.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Folder {
    Scripts,
    Snapshots,
}

impl Folder {
    fn key(self) -> &'static str {
        match self {
            Folder::Scripts => "scripts_dir",
            Folder::Snapshots => "snapshots_dir",
        }
    }
}

/// The folders picked in Settings → General (`None` = the workspace's own).
#[derive(Default)]
pub struct Folders {
    pub scripts: Option<std::path::PathBuf>,
    pub snapshots: Option<std::path::PathBuf>,
    /// A folder picker that's open, and the folder it's for.
    picking: Option<(Folder, crossbeam_channel::Receiver<Option<std::path::PathBuf>>)>,
}

impl Folders {
    pub fn load(store: &opentern_core::Store) -> Folders {
        let get = |f: Folder| {
            store
                .value(f.key())
                .filter(|v| !v.is_empty())
                .map(std::path::PathBuf::from)
        };
        Folders {
            scripts: get(Folder::Scripts),
            snapshots: get(Folder::Snapshots),
            picking: None,
        }
    }

    fn slot(&mut self, f: Folder) -> &mut Option<std::path::PathBuf> {
        match f {
            Folder::Scripts => &mut self.scripts,
            Folder::Snapshots => &mut self.snapshots,
        }
    }
}

/// Opens the system's folder picker without blocking the UI.
fn pick_folder(title: &str, start: &std::path::Path) -> crossbeam_channel::Receiver<Option<std::path::PathBuf>> {
    let (tx, rx) = crossbeam_channel::bounded(1);
    // Made here (some platforms need the UI thread), awaited on another.
    let fut = rfd::AsyncFileDialog::new()
        .set_title(title)
        .set_directory(start)
        .pick_folder();
    std::thread::spawn(move || {
        let _ = tx.send(pollster::block_on(fut).map(|h| h.path().to_path_buf()));
    });
    rx
}

pub struct SettingsPage {
    pub cat: Cat,
    providers: Option<Providers>,
    chart: Option<ChartSettings>,
    /// Its flows stop when Settings closes.
    pub(crate) transfer: Option<crate::transfer::Transfer>,
}

impl SettingsPage {
    pub fn new(cat: Cat) -> SettingsPage {
        SettingsPage {
            cat,
            providers: None,
            chart: None,
            transfer: None,
        }
    }
}

/// Set while the price cache is being cleared.
static CLEARING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
const CACHE_SIZE_ID: &str = "settings-cache-size";

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
        // Narrower categories when the window is (a 1000 px window has the
        // widget panel open beside this).
        let nav_w = if full.width() < 760.0 { 188.0 } else { 220.0 };
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
        c.add_space(14.0);
        // `OPENTERN_STILL=…,scroll-end` (screenshots): start at the bottom.
        let end = std::env::var("OPENTERN_STILL").is_ok_and(|v| v.split(',').any(|p| p == "scroll-end"));
        egui::ScrollArea::vertical()
            .id_salt(("settings", page.cat as u8))
            .auto_shrink([false; 2])
            .show(&mut c, |ui| {
                let w = (ui.available_width() - 14.0).min(720.0);
                ui.set_max_width(w);
                match page.cat {
                    Cat::General => self.general(ui, t),
                    Cat::Chart => page
                        .chart
                        .get_or_insert_with(|| ChartSettings::new(self))
                        .page(ui, self, t),
                    Cat::Providers => page
                        .providers
                        .get_or_insert_with(|| self.providers_form())
                        .page(ui, self, t),
                    Cat::Shortcuts => shortcuts(ui, t),
                    Cat::Transfer => page
                        .transfer
                        .get_or_insert_with(|| crate::transfer::Transfer::new(self))
                        .page(ui, self, t),
                    Cat::About => self.about(ui, t),
                }
                ui.add_space(24.0);
                if end {
                    ui.scroll_to_cursor(Some(egui::Align::BOTTOM));
                }
            });
        if !close && self.settings.is_none() {
            self.settings = Some(page);
        }
    }

    fn general(&mut self, ui: &mut Ui, t: &Theme) {
        self.poll_folder_pick();
        crate::dialogs::section(ui, t, "Folders");
        ui.add_space(4.0);
        self.folder_row(ui, t, Folder::Scripts);
        ui.add_space(8.0);
        self.folder_row(ui, t, Folder::Snapshots);
        ui.add_space(22.0);
        self.appearance(ui, t);
        ui.add_space(12.0);
        crate::dialogs::section(ui, t, "Symbol logos");
        ui.add_space(4.0);
        self.logo_settings(ui);
        ui.add_space(22.0);
        crate::dialogs::section(ui, t, "Large numbers");
        ui.add_space(4.0);
        self.number_settings(ui, t);
        ui.add_space(22.0);
        crate::dialogs::section(ui, t, "Price cache");
        ui.add_space(4.0);
        self.cache_settings(ui, t);
        ui.add_space(22.0);
        crate::dialogs::section(ui, t, "Alerts");
        ui.add_space(4.0);
        self.alert_settings(ui, t);
    }

    /// Keep alerts running with the window closed, and from login.
    fn alert_settings(&mut self, ui: &mut Ui, t: &Theme) {
        use crate::daemon::{AT_LOGIN, BACKGROUND};
        let mut background = crate::daemon::background_on(&self.store);
        if crate::widgets::checkbox(ui, &mut background, "Keep alerts running when Opentern is closed")
            .on_hover_text("Alerts are watched in the background until Opentern opens again.")
            .changed()
        {
            let _ = self.store.set_value(BACKGROUND, if background { "on" } else { "off" });
        }
        ui.add_space(8.0);
        let err_id = egui::Id::new("alerts-at-login-error");
        let mut at_login = self.store.value(AT_LOGIN).as_deref() == Some("on");
        let resp = ui.add_enabled_ui(background, |ui| {
            crate::widgets::checkbox(ui, &mut at_login, "Start watching at login")
                .on_hover_text("Alerts keep working after a restart, without opening Opentern.")
        });
        if resp.inner.changed() {
            let dir = self.store.dir().to_path_buf();
            let separate = dir != opentern_core::data_dir();
            match crate::daemon::set_at_login(&dir, separate, at_login) {
                Ok(()) => {
                    let _ = self.store.set_value(AT_LOGIN, if at_login { "on" } else { "off" });
                    ui.data_mut(|d| d.remove::<String>(err_id));
                }
                Err(e) => {
                    ui.data_mut(|d| {
                        d.insert_temp(err_id, format!("Couldn't change the login items: {e}"));
                    });
                }
            }
        }
        if let Some(e) = ui.data(|d| d.get_temp::<String>(err_id)) {
            ui.add_space(6.0);
            ui.label(egui::RichText::new(e).font(theme::regular(12.5)).color(t.text2));
        }
    }

    /// A picked folder arriving from the system's picker.
    fn poll_folder_pick(&mut self) {
        let Some((f, rx)) = &self.folders.picking else { return };
        let f = *f;
        match rx.try_recv() {
            Ok(Some(dir)) => {
                self.folders.picking = None;
                self.set_folder(f, Some(dir));
            }
            Ok(None) | Err(crossbeam_channel::TryRecvError::Disconnected) => self.folders.picking = None,
            Err(crossbeam_channel::TryRecvError::Empty) => {}
        }
    }

    /// Points `f` at `dir` (`None` = back inside the workspace) and saves it.
    pub(crate) fn set_folder(&mut self, f: Folder, dir: Option<std::path::PathBuf>) {
        // The workspace's own folder picked by hand is the default.
        let own = self.store.dir().join(match f {
            Folder::Scripts => "scripts",
            Folder::Snapshots => "snapshots",
        });
        let dir = dir.filter(|d| *d != own);
        let _ = self.store.set_value(
            f.key(),
            &dir.as_ref()
                .map(|d| d.to_string_lossy().into_owned())
                .unwrap_or_default(),
        );
        // Scripts on the charts look for their files there within a second.
        *self.folders.slot(f) = dir;
    }

    fn folder_row(&mut self, ui: &mut Ui, t: &Theme, f: Folder) {
        let (name, icon, dir) = match f {
            Folder::Scripts => ("Scripts", ph::CODE, self.scripts_dir()),
            Folder::Snapshots => ("Snapshots", ph::CAMERA, self.snapshots_dir()),
        };
        let custom = match f {
            Folder::Scripts => self.folders.scripts.is_some(),
            Folder::Snapshots => self.folders.snapshots.is_some(),
        };
        let h = 56.0;
        let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), h), Sense::hover());
        let p = ui.painter();
        p.rect_filled(r, CornerRadius::same(10), t.card);
        p.rect_stroke(
            r,
            CornerRadius::same(10),
            Stroke::new(1.0, t.border.gamma_multiply(0.7)),
            egui::StrokeKind::Inside,
        );
        p.text(
            pos2(r.left() + 18.0, r.center().y),
            Align2::LEFT_CENTER,
            icon,
            theme::regular(17.0),
            t.text2,
        );
        // Buttons on the right first; the path gets what's left.
        let mut right = r.right() - 10.0;
        let mut buttons = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(r.shrink2(vec2(10.0, 0.0)))
                .layout(egui::Layout::right_to_left(egui::Align::Center)),
        );
        buttons.spacing_mut().item_spacing.x = 4.0;
        if crate::widgets::secondary(&mut buttons, "Change").clicked() && self.folders.picking.is_none() {
            let _ = std::fs::create_dir_all(&dir);
            let title = format!("Choose a {} folder", name.to_lowercase());
            self.folders.picking = Some((f, pick_folder(&title, &dir)));
        }
        if crate::widgets::tool_button(&mut buttons, t, ph::FOLDER_OPEN, "Open folder", false, 30.0).clicked() {
            let _ = std::fs::create_dir_all(&dir);
            crate::app::open_path(&dir.join("."));
        }
        if custom
            && crate::widgets::tool_button(
                &mut buttons,
                t,
                ph::ARROW_COUNTER_CLOCKWISE,
                "Use the default folder",
                false,
                30.0,
            )
            .clicked()
        {
            self.set_folder(f, None);
        }
        right = right.min(buttons.min_rect().left() - 12.0);
        let text_l = r.left() + 46.0;
        let p = ui.painter().with_clip_rect(Rect::from_min_max(
            pos2(text_l, r.top()),
            pos2(right.max(text_l), r.bottom()),
        ));
        p.text(
            pos2(text_l, r.center().y - 9.0),
            Align2::LEFT_CENTER,
            name,
            theme::medium(13.5),
            t.text,
        );
        let path = shorten_home(&dir);
        let galley = ui.painter().layout_no_wrap(path.clone(), theme::regular(12.0), t.text3);
        let room = right - text_l;
        let shown = if galley.size().x > room {
            elide_middle(ui, &path, room)
        } else {
            path.clone()
        };
        p.text(
            pos2(text_l, r.center().y + 10.0),
            Align2::LEFT_CENTER,
            shown,
            theme::regular(12.0),
            t.text3,
        );
        let hit = Rect::from_min_max(pos2(text_l, r.top()), pos2(right.max(text_l), r.bottom()));
        ui.interact(hit, ui.id().with(("folder", f.key())), Sense::hover())
            .on_hover_text(dir.to_string_lossy());
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
        let per_row = ((avail + 10.0) / 150.0).floor().clamp(2.0, 4.0) as usize;
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

    /// Per market: millions and billions, or lakhs and crores (market
    /// caps, volumes).
    fn number_settings(&mut self, ui: &mut Ui, t: &Theme) {
        use crate::format::Numbers;
        use opentern_data::overview::Market;
        let opts = [
            (Numbers::Intl, "Millions, billions"),
            (Numbers::Indian, "Lakhs, crores"),
        ];
        let mut changed = false;
        for m in Market::ALL {
            ui.horizontal(|ui| {
                ui.set_min_height(34.0);
                let (r, _) = ui.allocate_exact_size(vec2(140.0, 30.0), Sense::hover());
                ui.painter().text(
                    r.left_center(),
                    Align2::LEFT_CENTER,
                    match m {
                        Market::India => "Indian stocks",
                        _ => m.label(),
                    },
                    theme::regular(13.0),
                    t.text,
                );
                let mut n = crate::format::numbers(m);
                if crate::widgets::segmented(ui, t, &mut n, &opts) {
                    crate::format::set_numbers(m, n);
                    changed = true;
                }
            });
        }
        if changed {
            let _ = self
                .store
                .set_value(crate::format::NUMBERS_KEY, &crate::format::numbers_json());
        }
    }

    /// The bar cache's size, and Clear (which asks first: `Modal::ClearCache`).
    /// The size is looked up every few seconds while shown.
    fn cache_settings(&mut self, ui: &mut Ui, t: &Theme) {
        let id = egui::Id::new(CACHE_SIZE_ID);
        let now = ui.input(|i| i.time);
        let size = match ui.data(|d| d.get_temp::<(f64, u64)>(id)) {
            Some((at, size)) if now - at < 3.0 => size,
            _ => {
                let size = opentern_data::cache::size(&self.store.dir().join("cache"));
                ui.data_mut(|d| d.insert_temp(id, (now, size)));
                size
            }
        };
        ui.ctx().request_repaint_after(std::time::Duration::from_secs(3));
        let clearing = CLEARING.load(std::sync::atomic::Ordering::Relaxed);
        ui.label(
            egui::RichText::new(if clearing {
                "Clearing…".to_string()
            } else {
                format!("{:.0} MB of prices on this computer", size as f64 / 1e6)
            })
            .color(t.text2),
        );
        ui.add_space(10.0);
        let clear = ui
            .add_enabled_ui(!clearing && !self.demo, |ui| {
                crate::widgets::secondary(ui, "Clear price cache")
            })
            .inner;
        if clear.clicked() {
            self.modal = Some(crate::dialogs::Modal::ClearCache);
        }
    }

    /// Empties the price cache off the UI thread (after the warning).
    pub(crate) fn clear_price_cache(&self, ctx: &egui::Context) {
        use std::sync::atomic::Ordering;
        CLEARING.store(true, Ordering::Relaxed);
        let dir = self.store.dir().join("cache");
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = opentern_data::cache::clear(&dir);
            CLEARING.store(false, Ordering::Relaxed);
            ctx.data_mut(|d| d.remove::<(f64, u64)>(egui::Id::new(CACHE_SIZE_ID)));
            ctx.request_repaint();
        });
    }

    fn logo_settings(&mut self, ui: &mut Ui) {
        let mut on = self.logos.enabled;
        if crate::widgets::checkbox(ui, &mut on, "Show symbol logos").changed() {
            self.logos.enabled = on;
            let _ = self.store.set_value("symbol_logos", if on { "on" } else { "off" });
        }
        ui.add_space(10.0);
        if crate::widgets::secondary(ui, "Clear saved logos").clicked() {
            self.logos.clear();
        }
    }

    fn about(&mut self, ui: &mut Ui, t: &Theme) {
        use crate::updater::Phase;
        // Logo, then the version and what Opentern is (wrapping in a narrow pane).
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 16.0;
            ui.add(egui::Image::new(&app_logo(ui.ctx())).fit_to_exact_size(vec2(56.0, 56.0)));
            ui.vertical(|ui| {
                ui.add_space(4.0);
                ui.label(
                    egui::RichText::new(format!("Opentern {}", env!("CARGO_PKG_VERSION")))
                        .font(theme::semibold(16.0))
                        .color(t.text),
                );
                ui.label(
                    egui::RichText::new("A fast, local-first market analysis terminal.")
                        .font(theme::regular(13.0))
                        .color(t.text2),
                );
            });
        });
        ui.add_space(18.0);

        crate::dialogs::section(ui, t, "Updates");
        ui.add_space(4.0);
        let subtle = |s: String| egui::RichText::new(s).font(theme::regular(12.5)).color(t.text2);
        let phase = self.updater.phase.clone();
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 12.0;
            match &phase {
                Phase::Available(r) => {
                    if crate::widgets::primary(ui, &format!("Update to {}", r.version)).clicked() {
                        self.update_action(ui.ctx());
                    }
                    ui.label(subtle("Update available".into()));
                }
                Phase::Manual(r, why) => {
                    if crate::widgets::secondary(ui, &format!("Download {}", r.version)).clicked() {
                        self.update_action(ui.ctx());
                    }
                    ui.label(subtle(format!("{why}.")));
                }
                Phase::Installing(r) => {
                    ui.label(subtle(format!("Updating to {}", r.version)));
                    crate::widgets::progress(
                        ui,
                        t,
                        egui::Id::new("settings-update-progress"),
                        vec2(160.0, 6.0),
                        self.updater.progress,
                    );
                    ui.label(subtle(format!("{:.0}%", self.updater.progress * 100.0)));
                }
                Phase::Ready(r) => {
                    if crate::widgets::primary(ui, "Restart Opentern").clicked() {
                        self.update_action(ui.ctx());
                    }
                    ui.label(subtle(format!(
                        "Opentern {} is installed. Restart to use it.",
                        r.version
                    )));
                }
                _ => {
                    let checking = phase == Phase::Checking;
                    let label = if checking { "Checking" } else { "Check for updates" };
                    if crate::widgets::secondary(ui, label).clicked() && !checking {
                        self.updater.check(ui.ctx(), true);
                    }
                    match &phase {
                        Phase::UpToDate => {
                            ui.label(subtle("Opentern is up to date".into()));
                        }
                        Phase::Failed(m) => {
                            ui.label(subtle(m.clone()));
                        }
                        _ => {}
                    }
                }
            }
        });
        ui.add_space(10.0);
        let mut on = self.store.settings().check_updates;
        if crate::widgets::checkbox(ui, &mut on, "Check for updates when Opentern starts").changed() {
            let s = opentern_core::store::Settings {
                check_updates: on,
                ..self.store.settings().clone()
            };
            if let Err(e) = self.store.set_settings(s) {
                crate::diag::crumb(format!("couldn't save the update setting: {e}"));
            }
        }
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            if crate::widgets::secondary(ui, "What's new").clicked() {
                self.modal = Some(crate::dialogs::Modal::WhatsNew(Box::new(
                    crate::whatsnew::Card::current(),
                )));
            }
            if crate::widgets::secondary(ui, "Release notes").clicked() {
                crate::app::open_url("https://github.com/ahaan-shah/opentern/releases");
            }
            if crate::widgets::secondary(ui, "Source code").clicked() {
                crate::app::open_url("https://github.com/ahaan-shah/opentern");
            }
        });
    }
}

/// The keyboard shortcuts sheet: TradingView's, plus Opentern's own.
fn shortcuts(ui: &mut Ui, t: &Theme) {
    use crate::chart::draw::Tool;
    let general: &[(&str, &str)] = &[
        ("Symbol search", "Type anywhere"),
        ("Change the interval", "Type a number"),
        ("Indicators", "/"),
        ("Create an alert", "Alt A"),
        ("Reset the chart view", "Alt R"),
        ("Save a snapshot", "Alt S"),
        ("Add to watchlist", "Alt W"),
        ("Market overview", "Alt M"),
        ("Full screen", "Shift F"),
        ("Undo", "Ctrl Z"),
        ("Redo", "Ctrl Y"),
        ("Step one bar", "← →"),
        ("Measure", "Shift Drag"),
        ("Hide all drawings", "Ctrl Alt H"),
        ("Delete the selected drawing", "Delete"),
        ("Cancel", "Esc"),
    ];
    let scale: &[(&str, &str)] = &[("Log", "Alt L"), ("Percent", "Alt P"), ("Auto", "Alt U")];
    let agent: &[(&str, &str)] = &[
        ("Send", "Enter"),
        ("New line", "Shift Enter"),
        (
            "Paste an image",
            if cfg!(target_os = "macos") { "Cmd V" } else { "Ctrl V" },
        ),
        ("Answer a prompt", "1 2 3"),
    ];
    crate::dialogs::section(ui, t, "General");
    for (what, keys) in general {
        shortcut_row(ui, t, None, what, keys);
    }
    ui.add_space(10.0);
    crate::dialogs::section(ui, t, "Price scale");
    for (what, keys) in scale {
        shortcut_row(ui, t, None, what, keys);
    }
    ui.add_space(10.0);
    crate::dialogs::section(ui, t, "Drawing tools");
    for tool in Tool::ALL {
        shortcut_row(ui, t, Some(tool.icon()), tool.name(), tool.shortcut());
    }
    ui.add_space(10.0);
    crate::dialogs::section(ui, t, "Agent panel");
    for (what, keys) in agent {
        shortcut_row(ui, t, None, what, keys);
    }
}

/// One row of the sheet: what it does on the left, its keys as keycaps on
/// the right ("Type anywhere" and the like as plain words).
fn shortcut_row(ui: &mut Ui, t: &Theme, icon: Option<&str>, what: &str, keys: &str) {
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 34.0), Sense::hover());
    let p = ui.painter();
    p.hline(
        r.x_range(),
        r.bottom() - 0.5,
        Stroke::new(1.0, t.border.gamma_multiply(0.5)),
    );
    let mut x = r.left() + 2.0;
    if let Some(i) = icon {
        p.text(
            pos2(x + 8.0, r.center().y),
            Align2::CENTER_CENTER,
            i,
            theme::regular(15.0),
            t.text2,
        );
        x += 26.0;
    }
    p.text(
        pos2(x, r.center().y),
        Align2::LEFT_CENTER,
        what,
        theme::regular(13.5),
        t.text,
    );
    let words = keys.starts_with("Type");
    if words {
        p.text(
            pos2(r.right() - 4.0, r.center().y),
            Align2::RIGHT_CENTER,
            keys,
            theme::regular(12.5),
            t.text2,
        );
        return;
    }
    // Keycaps, laid out right to left. Ctrl is Cmd on macOS.
    let mut right = r.right() - 2.0;
    for k in keys.split(' ').rev() {
        let k = if cfg!(target_os = "macos") && k == "Ctrl" {
            "Cmd"
        } else {
            k
        };
        let g = p.layout_no_wrap(k.to_string(), theme::medium(12.0), t.text);
        let w = (g.size().x + 14.0).max(24.0);
        let cap = Rect::from_min_size(pos2(right - w, r.center().y - 11.0), vec2(w, 22.0));
        p.rect(
            cap,
            CornerRadius::same(6),
            t.hover,
            Stroke::new(1.0, t.border),
            egui::StrokeKind::Inside,
        );
        // A slightly deeper bottom edge, like a key.
        p.hline(
            cap.x_range().shrink(4.0),
            cap.bottom() - 0.5,
            Stroke::new(1.0, t.border.lerp_to_gamma(t.text3, 0.3)),
        );
        p.galley(cap.center() - g.size() / 2.0, g, t.text);
        right = cap.left() - 4.0;
    }
}

/// The app icon as a texture, rendered once from the packaged SVG.
pub(crate) fn app_logo(ctx: &egui::Context) -> egui::TextureHandle {
    let id = egui::Id::new("opentern-app-logo");
    if let Some(tex) = ctx.data(|d| d.get_temp::<egui::TextureHandle>(id)) {
        return tex;
    }
    use resvg::{tiny_skia, usvg};
    const SIZE: u32 = 192;
    let mut img = egui::ColorImage::filled([SIZE as usize; 2], egui::Color32::TRANSPARENT);
    let svg = include_str!("../../../packaging/linux/opentern.svg");
    if let (Ok(tree), Some(mut pm)) = (
        usvg::Tree::from_str(svg, &usvg::Options::default()),
        tiny_skia::Pixmap::new(SIZE, SIZE),
    ) {
        let k = SIZE as f32 / tree.size().width().max(tree.size().height());
        resvg::render(&tree, tiny_skia::Transform::from_scale(k, k), &mut pm.as_mut());
        img = egui::ColorImage::from_rgba_premultiplied([SIZE as usize; 2], pm.data());
    }
    let tex = ctx.load_texture("opentern-app-logo", img, egui::TextureOptions::LINEAR);
    ctx.data_mut(|d| d.insert_temp(id, tex.clone()));
    tex
}

/// `~/…` for paths under the home folder (only for showing).
pub(crate) fn shorten_home(p: &std::path::Path) -> String {
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(std::path::PathBuf::from);
    match home.and_then(|h| p.strip_prefix(&h).ok().map(|r| r.to_path_buf())) {
        Some(rest) => format!("~{}{}", std::path::MAIN_SEPARATOR, rest.display()),
        None => p.display().to_string(),
    }
}

/// `text` cut in the middle with "…" to fit `width` at 12 pt.
fn elide_middle(ui: &Ui, text: &str, width: f32) -> String {
    let chars: Vec<char> = text.chars().collect();
    let fits = |s: &str| {
        ui.painter()
            .layout_no_wrap(s.to_string(), theme::regular(12.0), egui::Color32::WHITE)
            .size()
            .x
            <= width
    };
    let (mut lo, mut hi) = (0usize, chars.len());
    // Largest n whose head + … + tail fits, keeping more of the tail.
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        let head = mid / 3;
        let s: String = chars[..head]
            .iter()
            .chain(['…'].iter())
            .chain(chars[chars.len() - (mid - head)..].iter())
            .collect();
        if fits(&s) { lo = mid } else { hi = mid - 1 }
    }
    let head = lo / 3;
    chars[..head]
        .iter()
        .chain(['…'].iter())
        .chain(chars[chars.len() - (lo - head)..].iter())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::drive::Driver;

    #[test]
    fn clearing_the_price_cache_asks_first() {
        let mut d = Driver::new("clear-cache");
        let cache = d.dir.join("cache");
        let tf = opentern_core::Timeframe::parse("1D").unwrap();
        let bar = opentern_core::Bar {
            time: 0,
            open: 1.0,
            high: 1.0,
            low: 1.0,
            close: 1.0,
            volume: 1.0,
        };
        opentern_data::cache::Cache::open(&cache)
            .unwrap()
            .save("NASDAQ:AAPL", tf, &[bar])
            .unwrap();
        let held = |c: &std::path::Path| {
            opentern_data::cache::Cache::open(c)
                .unwrap()
                .last("NASDAQ:AAPL", tf, 5)
                .unwrap()
                .len()
        };
        // Cancel keeps it.
        d.app.modal = Some(crate::dialogs::Modal::ClearCache);
        d.frame(vec![]);
        let texts = d.texts();
        assert!(
            texts.iter().any(|t| t == "Clear the price cache?"),
            "{:?}",
            texts.iter().filter(|t| t.contains("lear")).collect::<Vec<_>>()
        );
        let cancel = d.text_rect("Cancel").expect("Cancel");
        d.click(cancel.center());
        d.frame(vec![]);
        assert!(d.app.modal.is_none());
        assert_eq!(held(&cache), 1);
        // The dialog stays a dialog's size (a bare right-to-left row in a
        // modal once took the whole window's height).
        d.app.modal = Some(crate::dialogs::Modal::ClearCache);
        let title = d.text_rect("Clear the price cache?").unwrap();
        let clear = d.text_rect("Clear cache").expect("Clear cache");
        assert!(clear.top() - title.top() < 300.0, "{title:?} {clear:?}");
        d.click(clear.center());
        d.frame(vec![]);
        assert!(d.app.modal.is_none());
        d.wait(|_| !CLEARING.load(std::sync::atomic::Ordering::Relaxed));
        assert_eq!(held(&cache), 0);
    }

    #[test]
    fn folders_move_scripts_and_snapshots_and_stay_moved() {
        let mut d = Driver::new("folders");
        let ws = d.dir.clone();
        let scripts = ws.with_file_name(format!("{}-my-scripts", ws.file_name().unwrap().to_string_lossy()));
        let snaps = ws.with_file_name(format!("{}-My Snapshots", ws.file_name().unwrap().to_string_lossy()));
        let _ = std::fs::remove_dir_all(&scripts);
        let _ = std::fs::remove_dir_all(&snaps);
        assert_eq!(d.app.scripts_dir(), ws.join("scripts"));
        assert_eq!(d.app.snapshots_dir(), ws.join("snapshots"));

        d.app.set_folder(Folder::Scripts, Some(scripts.clone()));
        d.app.set_folder(Folder::Snapshots, Some(snaps.clone()));
        assert_eq!(d.app.scripts_dir(), scripts);
        assert_eq!(d.app.snapshots_dir(), snaps);

        // The editor, the chart and the agent all read the new folder.
        std::fs::create_dir_all(&scripts).unwrap();
        std::fs::write(
            scripts.join("moved.pine"),
            "//@version=6\nindicator(\"Moved\")\nplot(close)\n",
        )
        .unwrap();
        assert_eq!(d.app.script_files(), ["moved.pine"]);
        assert!(
            d.app
                .claude_context()
                .contains(&format!("Scripts folder: {}.", scripts.display()))
        );

        // Snapshots save there.
        let img = egui::ColorImage::new([4, 4], vec![egui::Color32::WHITE; 16]);
        let png = d.app.save_png(&img).unwrap();
        assert_eq!(png.parent(), Some(snaps.as_path()));

        // The page shows them.
        d.app.open_settings(Cat::General);
        let texts = d.texts().join("\n");
        assert!(
            texts.contains("FOLDERS") && texts.contains("Scripts") && texts.contains("Snapshots"),
            "{texts}"
        );

        // Loaded again at the next start.
        let f = Folders::load(&opentern_core::Store::open(&ws).unwrap());
        assert_eq!(f.scripts.as_deref(), Some(scripts.as_path()));
        assert_eq!(f.snapshots.as_deref(), Some(snaps.as_path()));

        // Back to the default, by reset or by picking the workspace's own.
        d.app.set_folder(Folder::Scripts, None);
        d.app.set_folder(Folder::Snapshots, Some(ws.join("snapshots")));
        assert_eq!(d.app.store.value("scripts_dir").as_deref(), Some(""));
        assert_eq!(d.app.store.value("snapshots_dir").as_deref(), Some(""));
        assert_eq!(d.app.scripts_dir(), ws.join("scripts"));
        let _ = std::fs::remove_dir_all(&scripts);
        let _ = std::fs::remove_dir_all(&snaps);
    }
}

//! Settings: a page that takes the chart area (☰ or the gear opens it,
//! Esc or × closes it), with its categories down the left: General
//! (folders, theme, symbol logos), Chart, Data providers, Keyboard
//! shortcuts, Export and import, and About.

use crate::app::App;
use crate::dialogs::ChartSettings;
use crate::dialogs::providers::Providers;
use crate::i18n::{msg, nt, tr, trc, trf, trm};
use crate::icons::ph;
use crate::motion;
use crate::theme::{self, THEMES, Theme};
use egui::{Align2, CornerRadius, Rect, Sense, Stroke, Ui, pos2, vec2};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Cat {
    General,
    Chart,
    Drawings,
    Providers,
    Shortcuts,
    Transfer,
    About,
}

impl Cat {
    pub const ALL: [Cat; 7] = [
        Cat::General,
        Cat::Chart,
        Cat::Drawings,
        Cat::Providers,
        Cat::Shortcuts,
        Cat::Transfer,
        Cat::About,
    ];

    fn title(self) -> &'static str {
        match self {
            Cat::General => nt!("General"),
            Cat::Chart => nt!("Chart"),
            Cat::Drawings => nt!("Drawings"),
            Cat::Providers => nt!("Data providers"),
            Cat::Shortcuts => nt!("Keyboard shortcuts"),
            Cat::Transfer => nt!("Export and import"),
            Cat::About => nt!("About"),
        }
    }

    fn icon(self) -> &'static str {
        match self {
            Cat::General => ph::SLIDERS_HORIZONTAL,
            Cat::Chart => ph::CHART_LINE,
            Cat::Drawings => ph::PENCIL_SIMPLE_LINE,
            Cat::Providers => ph::DATABASE,
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
    /// The shortcut waiting for its new keys.
    pub(crate) recording: Option<crate::keys::Action>,
}

impl SettingsPage {
    pub fn new(cat: Cat) -> SettingsPage {
        SettingsPage {
            cat,
            providers: None,
            chart: None,
            transfer: None,
            recording: None,
        }
    }
}

/// Set while the price cache is being cleared.
static CLEARING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
const CACHE_SIZE_ID: &str = "settings-cache-size";

/// Settings → Drawings: what a tool does once a drawing is placed
/// ("cursor": back to the cursor; else it stays picked).
pub const AFTER_DRAWING: &str = "after_drawing";

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
        let titles = Cat::ALL.map(|c| tr(c.title()));
        let nav_w = crate::widgets::widest(ui, theme::medium(13.5), titles) + 72.0;
        let nav_w = nav_w.max(if full.width() < 760.0 { 188.0 } else { 220.0 });
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
            egui::RichText::new(tr("Settings"))
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
                tr(cat.title()),
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
        if crate::widgets::tool_button(&mut c, t, ph::X, "", false, 30.0).clicked() {
            close = true;
        }
        let inner = Rect::from_min_max(
            pos2(body.left() + 32.0, body.top() + 18.0),
            pos2(body.right() - 24.0, body.bottom() - 8.0),
        );
        let mut c = ui.new_child(egui::UiBuilder::new().max_rect(inner));
        c.set_clip_rect(inner.intersect(ui.clip_rect()));
        c.label(
            egui::RichText::new(tr(page.cat.title()))
                .font(theme::semibold(18.0))
                .color(t.text),
        );
        c.add_space(14.0);
        // `OPENTERN_STILL=…,scroll-end` (screenshots): start at the bottom.
        let end = std::env::var("OPENTERN_STILL").is_ok_and(|v| v.split(',').any(|p| p == "scroll-end"));
        // `scroll:<points>`: start that far down (the themes, past Folders).
        let down: Option<f32> = std::env::var("OPENTERN_STILL")
            .ok()
            .and_then(|v| v.split(',').find_map(|p| p.strip_prefix("scroll:")?.parse().ok()));
        let mut area = egui::ScrollArea::vertical()
            .id_salt(("settings", page.cat as u8))
            .auto_shrink([false; 2]);
        if let Some(y) = down {
            area = area.vertical_scroll_offset(y);
        }
        area.show(&mut c, |ui| {
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
                Cat::Drawings => self.drawings_page(ui, t),
                Cat::Shortcuts => self.shortcuts_page(ui, t, &mut page.recording),
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
        crate::dialogs::section(ui, t, tr("Language"));
        ui.add_space(4.0);
        self.language_settings(ui);
        ui.add_space(22.0);
        crate::dialogs::section(ui, t, tr("Folders"));
        ui.add_space(4.0);
        self.folder_row(ui, t, Folder::Scripts);
        ui.add_space(8.0);
        self.folder_row(ui, t, Folder::Snapshots);
        ui.add_space(22.0);
        self.appearance(ui, t);
        ui.add_space(12.0);
        crate::dialogs::section(ui, t, tr("Symbol logos"));
        ui.add_space(4.0);
        self.logo_settings(ui);
        ui.add_space(22.0);
        crate::dialogs::section(ui, t, tr("Large numbers"));
        ui.add_space(4.0);
        self.number_settings(ui, t);
        ui.add_space(22.0);
        crate::dialogs::section(ui, t, tr("Price cache"));
        ui.add_space(4.0);
        self.cache_settings(ui, t);
        ui.add_space(22.0);
        crate::dialogs::section(ui, t, tr("Alerts"));
        ui.add_space(4.0);
        self.alert_settings(ui, t);
    }

    /// What happens to alerts when Opentern is closed: one plain choice
    /// (pause them; keep checking them; keep checking them, also after a
    /// restart), instead of two linked checkboxes.
    fn alert_settings(&mut self, ui: &mut Ui, t: &Theme) {
        use crate::daemon::{AT_LOGIN, BACKGROUND};
        let background = crate::daemon::background_on(&self.store);
        let at_login = self.store.value(AT_LOGIN).as_deref() == Some("on");
        let was: u8 = match (background, at_login) {
            (false, _) => 0,
            (true, false) => 1,
            (true, true) => 2,
        };
        let mut pick = was;
        ui.label(egui::RichText::new(tr("When Opentern is closed")).color(t.text2));
        ui.add_space(4.0);
        let choices: [(u8, &str, &str); 3] = [
            (0, nt!("Pause alerts"), nt!("Alerts only fire while Opentern is open.")),
            (
                1,
                nt!("Keep checking alerts"),
                nt!(
                    "A small helper keeps checking your alerts and notifies you. It stops when you open \
                 Opentern again, or when no alert is left."
                ),
            ),
            (
                2,
                nt!("Keep checking, even after a restart"),
                nt!(
                    "The same, and it also starts when you log in to your computer, so alerts keep \
                 working after a restart."
                ),
            ),
        ];
        for (k, label, tip) in choices {
            crate::widgets::radio(ui, &mut pick, k, tr(label)).on_hover_text(tr(tip));
            ui.add_space(2.0);
        }
        let err_id = egui::Id::new("alerts-at-login-error");
        if pick != was {
            let _ = self.store.set_value(BACKGROUND, if pick > 0 { "on" } else { "off" });
            let want_login = pick == 2;
            if want_login != at_login {
                let dir = self.store.dir().to_path_buf();
                let separate = dir != opentern_core::data_dir();
                match crate::daemon::set_at_login(&dir, separate, want_login) {
                    Ok(()) => {
                        let _ = self.store.set_value(AT_LOGIN, if want_login { "on" } else { "off" });
                        ui.data_mut(|d| d.remove::<String>(err_id));
                    }
                    Err(e) => {
                        ui.data_mut(|d| {
                            d.insert_temp(err_id, msg!("Couldn't set Opentern to start at login: {e}"));
                        });
                    }
                }
            }
        }
        if let Some(e) = ui.data(|d| d.get_temp::<String>(err_id)) {
            ui.add_space(6.0);
            ui.label(egui::RichText::new(trm(&e)).font(theme::regular(12.5)).color(t.text2));
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
            Folder::Scripts => (nt!("Scripts"), ph::CODE, self.scripts_dir()),
            Folder::Snapshots => (nt!("Snapshots"), ph::CAMERA, self.snapshots_dir()),
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
        if crate::widgets::secondary(&mut buttons, trc("button", "Change")).clicked() && self.folders.picking.is_none()
        {
            let _ = std::fs::create_dir_all(&dir);
            let title = match f {
                Folder::Scripts => tr("Choose a scripts folder"),
                Folder::Snapshots => tr("Choose a snapshots folder"),
            };
            self.folders.picking = Some((f, pick_folder(title, &dir)));
        }
        if crate::widgets::tool_button(&mut buttons, t, ph::FOLDER_OPEN, tr("Open folder"), false, 30.0).clicked() {
            let _ = std::fs::create_dir_all(&dir);
            crate::app::open_path(&dir.join("."));
        }
        if custom
            && crate::widgets::tool_button(
                &mut buttons,
                t,
                ph::ARROW_COUNTER_CLOCKWISE,
                tr("Use the default folder"),
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
            tr(name),
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
        crate::dialogs::section(ui, t, tr("Theme"));
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
        ui.add_space(12.0);
        crate::dialogs::section(ui, t, tr("Font"));
        ui.add_space(4.0);
        self.font_picker(ui);
    }

    /// The interface's language: the system's, or one picked here.
    fn language_settings(&mut self, ui: &mut Ui) {
        use crate::i18n::{self, Lang};
        let saved = self.store.value(i18n::KEY).and_then(|c| Lang::from_code(&c));
        let name = |l: Option<Lang>| match l {
            Some(l) => l.name().to_string(),
            None => trf!("System ({})", Lang::system().name()),
        };
        let mut pick = saved;
        crate::widgets::combo("settings-language")
            .width(220.0)
            .selected_text(name(saved))
            .show_ui(ui, |ui| {
                for l in std::iter::once(None).chain(Lang::ALL.map(Some)) {
                    crate::widgets::option_value(ui, &mut pick, l, name(l));
                }
            });
        if pick != saved {
            if let Err(e) = self.store.set_value(i18n::KEY, pick.map_or("system", Lang::code)) {
                crate::diag::crumb(format!("couldn't save the language: {e}"));
            }
            self.set_language(pick.unwrap_or_else(Lang::system), ui.ctx());
        }
    }

    pub(crate) fn set_language(&mut self, lang: crate::i18n::Lang, ctx: &egui::Context) {
        crate::i18n::set(lang);
        // Chinese and Japanese draw the same characters differently.
        theme::install_fonts(ctx, &self.store.settings().font);
        self.paper_lines();
        ctx.request_repaint();
    }

    /// The interface font, each shown in its own face.
    fn font_picker(&mut self, ui: &mut Ui) {
        let current = theme::font_by_name(&self.store.settings().font).name;
        let face = |name: &str| egui::RichText::new(name).family(theme::preview_family(name)).size(13.5);
        let mut pick = None;
        crate::widgets::combo("settings-font")
            .width(220.0)
            .selected_text(face(current))
            .show_ui(ui, |ui| {
                for f in theme::FONTS {
                    if crate::widgets::option(ui, f.name == current, face(f.name)).clicked() {
                        pick = Some(f.name);
                    }
                }
            });
        if let Some(name) = pick.filter(|n| *n != current) {
            theme::install_fonts(ui.ctx(), name);
            let s = opentern_core::Settings {
                font: name.into(),
                ..self.store.settings().clone()
            };
            if let Err(e) = self.store.set_settings(s) {
                crate::diag::crumb(format!("couldn't save the font: {e}"));
            }
        }
    }

    /// Per market: millions and billions, or lakhs and crores (market
    /// caps, volumes).
    fn number_settings(&mut self, ui: &mut Ui, t: &Theme) {
        use crate::format::Numbers;
        use opentern_data::overview::Market;
        let opts = [
            (Numbers::Intl, tr("Millions, billions")),
            (Numbers::Indian, tr("Lakhs, crores")),
        ];
        let mut changed = false;
        for m in Market::ALL {
            ui.horizontal(|ui| {
                ui.set_min_height(34.0);
                let (r, _) = ui.allocate_exact_size(vec2(140.0, 30.0), Sense::hover());
                ui.painter().text(
                    r.left_center(),
                    Align2::LEFT_CENTER,
                    tr(m.label()),
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
                tr("Clearing…").to_string()
            } else {
                trf!("{:.0} MB of prices on this computer", size as f64 / 1e6)
            })
            .color(t.text2),
        );
        ui.add_space(10.0);
        let clear = ui
            .add_enabled_ui(!clearing && !self.demo, |ui| {
                crate::widgets::secondary(ui, tr("Clear price cache"))
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
        if crate::widgets::checkbox(ui, &mut on, tr("Show symbol logos")).changed() {
            self.logos.enabled = on;
            let _ = self.store.set_value("symbol_logos", if on { "on" } else { "off" });
        }
        ui.add_space(10.0);
        if crate::widgets::secondary(ui, tr("Clear saved logos")).clicked() {
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
                    egui::RichText::new(trf!("Opentern {}", env!("CARGO_PKG_VERSION")))
                        .font(theme::semibold(16.0))
                        .color(t.text),
                );
                ui.label(
                    egui::RichText::new(tr("A fast, local-first market analysis terminal."))
                        .font(theme::regular(13.0))
                        .color(t.text2),
                );
            });
        });
        ui.add_space(18.0);

        crate::dialogs::section(ui, t, tr("Updates"));
        ui.add_space(4.0);
        let subtle = |s: String| egui::RichText::new(s).font(theme::regular(12.5)).color(t.text2);
        let phase = self.updater.phase.clone();
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 12.0;
            match &phase {
                Phase::Available(r) => {
                    if crate::widgets::primary(ui, &trf!("Update to {}", r.version)).clicked() {
                        self.update_action(ui.ctx());
                    }
                    ui.label(subtle(tr("Update available").into()));
                }
                Phase::Manual(r, why) => {
                    if crate::widgets::secondary(ui, &trf!("Download {}", r.version)).clicked() {
                        self.update_action(ui.ctx());
                    }
                    ui.label(subtle(format!("{}.", trm(why))));
                }
                Phase::Installing(r) => {
                    use opentern_core::update::Step;
                    let step = self.updater.step;
                    ui.label(subtle(match step {
                        Step::Downloading(_) => trf!("Downloading {}", r.version),
                        Step::Retrying => tr("Connection dropped, trying again").into(),
                        Step::Checking => tr("Checking the download").into(),
                        Step::Installing => trf!("Installing {}", r.version),
                    }));
                    crate::widgets::progress(
                        ui,
                        t,
                        egui::Id::new("settings-update-progress"),
                        vec2(160.0, 6.0),
                        self.updater.progress,
                    );
                    if matches!(step, Step::Downloading(_) | Step::Retrying) {
                        ui.label(subtle(format!("{:.0}%", self.updater.progress * 100.0)));
                    } else {
                        ui.add(egui::Spinner::new().size(13.0).color(t.text3));
                    }
                }
                Phase::Retry(_, why) => {
                    if crate::widgets::primary(ui, tr("Try again")).clicked() {
                        self.update_action(ui.ctx());
                    }
                    ui.label(subtle(trm(why).into_owned()));
                }
                Phase::Ready(r) => {
                    if crate::widgets::primary(ui, tr("Restart Opentern")).clicked() {
                        self.update_action(ui.ctx());
                    }
                    ui.label(subtle(trf!("Opentern {} is installed. Restart to use it.", r.version)));
                }
                _ => {
                    let checking = phase == Phase::Checking;
                    let label = if checking {
                        tr("Checking")
                    } else {
                        tr("Check for updates")
                    };
                    if crate::widgets::secondary(ui, label).clicked() && !checking {
                        self.updater.check(ui.ctx(), true);
                    }
                    match &phase {
                        Phase::UpToDate => {
                            ui.label(subtle(tr("Opentern is up to date").into()));
                        }
                        Phase::Failed(m) => {
                            ui.label(subtle(trm(m).into_owned()));
                        }
                        _ => {}
                    }
                }
            }
        });
        ui.add_space(10.0);
        let mut on = self.store.settings().check_updates;
        if crate::widgets::checkbox(ui, &mut on, tr("Check for updates when Opentern starts")).changed() {
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
            if crate::widgets::secondary(ui, tr("What's new")).clicked() {
                self.modal = Some(crate::dialogs::Modal::WhatsNew(Box::new(
                    crate::whatsnew::Card::current(),
                )));
            }
            if crate::widgets::secondary(ui, tr("Release notes")).clicked() {
                crate::app::open_url("https://github.com/ahaan-shah/opentern/releases");
            }
            if crate::widgets::secondary(ui, tr("Source code")).clicked() {
                crate::app::open_url("https://github.com/ahaan-shah/opentern");
            }
        });
    }
}

impl App {
    /// The keyboard shortcuts sheet: TradingView's, plus Opentern's own.
    /// A pencil (or a click on the keys) waits for new keys: Delete takes
    /// them off, Esc keeps what was there. A changed one gets a reset
    /// beside it; keys two shortcuts share are edged in orange with a "!".
    /// Keys that belong to typing or a place are listed but fixed
    /// (`keys.rs`).
    /// Settings → Drawings: what happens after a drawing, and the look
    /// every tool's new drawings start with (the same defaults the drawing
    /// bar's star saves).
    fn drawings_page(&mut self, ui: &mut Ui, t: &Theme) {
        use crate::chart::draw::{DrawStyle, Group, Tool};
        crate::dialogs::section(ui, t, tr("After drawing"));
        ui.add_space(4.0);
        let mut back = self.back_to_cursor;
        crate::widgets::radio(ui, &mut back, false, tr("Keep the tool"))
            .on_hover_text(tr("Draw again with the same tool until you pick another or press Esc."));
        ui.add_space(2.0);
        crate::widgets::radio(ui, &mut back, true, tr("Back to the cursor"))
            .on_hover_text(tr("Each drawing is one go; pick the tool again for the next."));
        if back != self.back_to_cursor {
            self.back_to_cursor = back;
            let _ = self
                .store
                .set_value(AFTER_DRAWING, if back { "cursor" } else { "keep" });
        }
        ui.add_space(22.0);
        crate::dialogs::section(ui, t, tr("Default look"));
        // "Reset all" at the heading's right, once any tool has its own look.
        let head = ui.min_rect();
        if !self.draw_styles.is_empty() {
            let r = Rect::from_min_max(
                pos2(head.right() - 120.0, head.bottom() - 34.0),
                pos2(head.right(), head.bottom()),
            );
            let mut c = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(r)
                    .layout(egui::Layout::right_to_left(egui::Align::Center)),
            );
            if w::secondary(&mut c, tr("Reset all")).clicked() {
                let tools: Vec<Tool> = self.draw_styles.keys().copied().collect();
                for tool in tools {
                    self.reset_drawing_default(tool);
                }
            }
        }
        ui.add_space(4.0);
        for g in Group::ALL.into_iter().filter(|g| *g != Group::Cursors) {
            ui.add_space(10.0);
            ui.label(
                egui::RichText::new(tr(g.name()))
                    .font(theme::medium(12.0))
                    .color(t.text3),
            );
            ui.add_space(2.0);
            for &tool in g.tools() {
                let saved = self.draw_styles.get(&tool).cloned();
                let mut style = saved.clone().unwrap_or_else(|| DrawStyle::for_tool(tool));
                let was = style.clone();
                let mut reset = false;
                ui.push_id(tool.key(), |ui| {
                    let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), 38.0), Sense::hover());
                    let hover = ui.rect_contains_pointer(row);
                    ui.painter().rect_filled(
                        row,
                        CornerRadius::same(7),
                        motion::with_alpha(t.hover_wash(), if hover { 1.0 } else { 0.0 }),
                    );
                    // The controls at the right edge (as the drawing bar has
                    // them; their width is last frame's), a sample of the look
                    // left of them, and the name in what's left.
                    let w_id = egui::Id::new(("drawing-default-w", tool.key()));
                    let width: f32 = ui.data(|d| d.get_temp(w_id)).unwrap_or(200.0);
                    let right = row.right() - 4.0;
                    if saved.is_some() {
                        let r = Rect::from_center_size(pos2(right - 14.0, row.center().y), vec2(28.0, 28.0));
                        let resp = ui.interact(r, ui.id().with("reset"), Sense::click());
                        reset = w::tool_button_look(
                            ui,
                            t,
                            resp,
                            ph::ARROW_COUNTER_CLOCKWISE,
                            tr("Reset to default"),
                            false,
                        )
                        .clicked();
                    }
                    let left = right - 34.0 - width;
                    let mut c = ui.new_child(
                        egui::UiBuilder::new()
                            .max_rect(Rect::from_min_max(
                                pos2(left, row.top()),
                                pos2(right - 34.0, row.bottom()),
                            ))
                            .layout(egui::Layout::left_to_right(egui::Align::Center)),
                    );
                    c.spacing_mut().item_spacing.x = 6.0;
                    let fills = matches!(
                        tool,
                        Tool::Rectangle
                            | Tool::Ellipse
                            | Tool::Triangle
                            | Tool::ParallelChannel
                            | Tool::PriceRange
                            | Tool::DateRange
                            | Tool::DatePriceRange
                    );
                    {
                        let c = &mut c;
                        crate::dialogs::color::color_button(c, t, &mut style.color);
                        if fills {
                            crate::dialogs::color::color_button(c, t, &mut style.fill);
                        }
                        if tool == Tool::Text {
                            crate::drawbar::text_size(c, &mut style.font);
                            if w::tool_button(c, t, ph::TEXT_B, tr("Bold"), style.bold, 28.0).clicked() {
                                style.bold = !style.bold;
                            }
                        } else if !matches!(tool, Tool::ArrowUp | Tool::ArrowDown) {
                            crate::drawbar::width_and_line(c, &mut style);
                        }
                    }
                    let used = c.min_rect().width();
                    if (used - width).abs() > 0.5 {
                        ui.data_mut(|d| d.insert_temp(w_id, used));
                        ui.ctx().request_repaint();
                    }
                    let controls_left = left;
                    let sample = Rect::from_min_max(
                        pos2(controls_left - 58.0, row.center().y - 9.0),
                        pos2(controls_left - 14.0, row.center().y + 9.0),
                    );
                    let p = ui.painter();
                    let col = crate::chart::style::c32(style.color);
                    if tool == Tool::Text {
                        let font = style.text_font();
                        p.text(
                            sample.center(),
                            Align2::CENTER_CENTER,
                            "Aa",
                            egui::FontId::new(font.size.min(18.0), font.family),
                            col,
                        );
                    } else {
                        if fills && style.fill.a() > 0 {
                            p.rect_filled(sample, CornerRadius::same(3), crate::chart::style::c32(style.fill));
                        }
                        crate::chart::paint::styled_hline(
                            p,
                            sample.left(),
                            sample.right(),
                            sample.center().y,
                            Stroke::new(style.width.max(1) as f32, col),
                            style.line,
                        );
                    }
                    p.text(
                        pos2(row.left() + 16.0, row.center().y),
                        Align2::CENTER_CENTER,
                        tool.icon(),
                        theme::regular(15.0),
                        t.text2,
                    );
                    let font = theme::regular(13.0);
                    let full = tr(tool.name());
                    let name = crate::format::elide_to(p, full, &font, sample.left() - row.left() - 46.0);
                    let cut = name != full;
                    let at = p.text(
                        pos2(row.left() + 34.0, row.center().y),
                        Align2::LEFT_CENTER,
                        name,
                        font,
                        t.text,
                    );
                    if cut {
                        ui.interact(at, ui.id().with("name"), Sense::hover())
                            .on_hover_text(full);
                    }
                });
                if reset {
                    self.reset_drawing_default(tool);
                } else if style != was {
                    self.save_drawing_default(tool, &style);
                }
            }
        }
    }

    fn shortcuts_page(&mut self, ui: &mut Ui, t: &Theme, recording: &mut Option<Action>) {
        use crate::chart::draw::Tool;
        // The next chord goes to the row that's waiting for it.
        if let Some(a) = *recording {
            ui.ctx().request_repaint();
            let events = ui.input(|i| i.events.clone());
            for e in events {
                let egui::Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } = e
                else {
                    continue;
                };
                let chord = Chord::new(key, modifiers);
                let new = match key {
                    egui::Key::Escape => {
                        *recording = None;
                        break;
                    }
                    egui::Key::Delete | egui::Key::Backspace if modifiers.is_none() => None,
                    _ if chord.usable() => Some(chord),
                    _ => continue,
                };
                self.keys.set(a, new);
                self.keys.save(&mut self.store);
                *recording = None;
                break;
            }
        }
        let fixed = |what: &'static str, keys: &'static str| (what, keys);
        let general_first = [
            fixed(nt!("Symbol search"), nt!("Type anywhere")),
            fixed(nt!("Change the interval"), nt!("Type a number")),
            fixed(nt!("Indicators"), "/"),
        ];
        let general_last = [
            fixed(nt!("Step one bar"), "← →"),
            fixed(nt!("Measure"), "Shift Drag"),
            fixed(nt!("Delete the selected drawing"), nt!("Delete")),
            fixed(nt!("Cancel"), "Esc"),
        ];
        let agent = [
            fixed(nt!("Send"), "Enter"),
            fixed(nt!("New line"), "Shift Enter"),
            fixed(
                nt!("Paste an image"),
                if cfg!(target_os = "macos") { "Cmd V" } else { "Ctrl V" },
            ),
            fixed(nt!("Answer a prompt"), "1 2 3"),
        ];
        let mut clicked = false;
        crate::dialogs::section(ui, t, tr("General"));
        ui.spacing_mut().item_spacing.y = 0.0;
        for (what, keys) in general_first {
            shortcut_row(ui, t, None, tr(what), keys);
        }
        for a in Action::GENERAL {
            clicked |= self.shortcut_edit_row(ui, t, None, a, recording);
        }
        for (what, keys) in general_last {
            shortcut_row(ui, t, None, tr(what), keys);
        }
        ui.add_space(18.0);
        crate::dialogs::section(ui, t, tr("Price scale"));
        for a in Action::SCALE {
            clicked |= self.shortcut_edit_row(ui, t, None, a, recording);
        }
        ui.add_space(18.0);
        crate::dialogs::section(ui, t, tr("Drawing tools"));
        shortcut_row(ui, t, Some(Tool::Cross.icon()), tr(Tool::Cross.name()), "Esc");
        for tool in Tool::ALL.into_iter().filter(|t| *t != Tool::Cross) {
            clicked |= self.shortcut_edit_row(ui, t, Some(tool.icon()), Action::Tool(tool), recording);
        }
        ui.add_space(18.0);
        crate::dialogs::section(ui, t, tr("Agent panel"));
        for (what, keys) in agent {
            shortcut_row(ui, t, None, tr(what), keys);
        }
        // A click anywhere else stops waiting.
        if !clicked && recording.is_some() && ui.input(|i| i.pointer.any_click()) {
            *recording = None;
        }
    }

    /// A row whose keys can be changed. Returns whether one of its buttons
    /// (or its keys) was clicked.
    fn shortcut_edit_row(
        &mut self,
        ui: &mut Ui,
        t: &Theme,
        icon: Option<&str>,
        a: Action,
        recording: &mut Option<Action>,
    ) -> bool {
        let waiting = *recording == Some(a);
        let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::hover());
        let line = row_text(ui, t, r, icon, tr(a.name()));
        let clashes = self.keys.clashes(a);
        let keys_end = line.right() - 4.0;
        let cy = r.center().y;

        // The keys (or the wait for them), right-aligned before the gutter;
        // a clash leaves room for its "!".
        let mark_w = if clashes.is_empty() || waiting { 0.0 } else { 18.0 };
        let caps = if waiting {
            let now = ui.input(|i| i.time);
            let held = ui.input(|i| i.modifiers);
            let mut words: Vec<&str> = Vec::new();
            if held.command {
                words.push(if cfg!(target_os = "macos") { "Cmd" } else { "Ctrl" });
            }
            if held.alt {
                words.push("Alt");
            }
            if held.shift {
                words.push("Shift");
            }
            // The modifiers held so far, then a pulsing slot for the key.
            let pulse = 0.5 + 0.5 * ((now * 4.0).sin() as f32);
            let slot = waiting_slot(ui, t, keys_end, cy, pulse);
            let left = keycaps(ui, t, slot.left() - CAP_GAP, cy, &words.join(" "), Cap::Lit);
            Rect::from_min_max(pos2(left, slot.top()), slot.max)
        } else {
            let label = self.keys.label(a);
            let end = keys_end - mark_w;
            if label.is_empty() {
                let g = ui.painter().layout_no_wrap("—".into(), theme::regular(14.0), t.text3);
                let at = pos2(end - g.size().x - 6.0, cy - g.size().y / 2.0);
                let rect = Rect::from_min_size(at, g.size());
                ui.painter().galley(at, g, t.text3);
                rect
            } else {
                let style = if clashes.is_empty() { Cap::Plain } else { Cap::Clash };
                let left = keycaps(ui, t, end, cy, &label, style);
                Rect::from_min_max(pos2(left, cy - CAP_H / 2.0), pos2(end, cy + CAP_H / 2.0))
            }
        };
        if mark_w > 0.0 {
            let m = Rect::from_center_size(pos2(keys_end - mark_w / 2.0 + 2.0, cy), vec2(mark_w, CAP_H));
            ui.painter()
                .text(m.center(), Align2::CENTER_CENTER, "!", theme::semibold(17.0), t.warn);
            let names: Vec<&str> = clashes.iter().map(|b| tr(b.name())).collect();
            ui.interact(m, ui.id().with(("clash", a)), Sense::hover())
                .on_hover_text(trf!("Also used by {}", names.join(", ")));
        }
        // The keys themselves can be clicked too.
        let on_keys = ui
            .interact(caps.expand(3.0), ui.id().with(("keys", a)), Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand);

        // Outside the row, on the right: edit, and reset once changed.
        let edit_r = Rect::from_center_size(pos2(line.right() + GUTTER_PAD + BTN / 2.0, cy), vec2(BTN, BTN));
        let reset_r = edit_r.translate(vec2(BTN + 4.0, 0.0));
        let edit = ui.interact(edit_r, ui.id().with(("edit", a)), Sense::click());
        let edit = w::tool_button_look(ui, t, edit, ph::PENCIL_SIMPLE, trc("button", "Change"), waiting);
        let mut reset = false;
        if !self.keys.is_default(a) {
            let rr = ui.interact(reset_r, ui.id().with(("reset", a)), Sense::click());
            reset = w::tool_button_look(ui, t, rr, ph::ARROW_COUNTER_CLOCKWISE, tr("Reset"), false).clicked();
        }
        if reset {
            self.keys.reset(a);
            self.keys.save(&mut self.store);
            *recording = None;
            return true;
        }
        if edit.clicked() || on_keys.clicked() {
            *recording = if waiting { None } else { Some(a) };
            return true;
        }
        false
    }
}

use crate::keys::{Action, Chord};
use crate::widgets as w;

/// A sheet row's height, its keycaps' height and the gap between them.
const ROW_H: f32 = 46.0;
const CAP_H: f32 = 28.0;
const CAP_GAP: f32 = 5.0;
/// The edit and reset buttons, right of the rows' lines.
const BTN: f32 = 28.0;
const GUTTER_PAD: f32 = 10.0;
const GUTTER: f32 = GUTTER_PAD + BTN + 4.0 + BTN;

/// How a keycap looks.
#[derive(Clone, Copy, PartialEq)]
enum Cap {
    Plain,
    /// Shared with another shortcut: edged in orange.
    Clash,
    /// Held while new keys are being pressed.
    Lit,
}

/// One fixed row of the sheet: what it does on the left, its keys as
/// keycaps on the right ("Type anywhere" and the like as plain words).
fn shortcut_row(ui: &mut Ui, t: &Theme, icon: Option<&str>, what: &str, keys: &str) {
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::hover());
    let line = row_text(ui, t, r, icon, what);
    let end = line.right() - 4.0;
    if keys.starts_with("Type") {
        ui.painter().text(
            pos2(end - 6.0, r.center().y),
            Align2::RIGHT_CENTER,
            tr(keys),
            theme::regular(13.0),
            t.text2,
        );
        return;
    }
    keycaps(ui, t, end, r.center().y, keys, Cap::Plain);
}

/// A sheet row's rule, icon and words. The rule stops short of the gutter
/// (where a row's buttons go); returns the part left of it.
fn row_text(ui: &Ui, t: &Theme, r: Rect, icon: Option<&str>, what: &str) -> Rect {
    let line = Rect::from_min_max(r.min, pos2(r.right() - GUTTER, r.bottom()));
    let p = ui.painter();
    p.hline(
        line.x_range(),
        line.bottom() - 0.5,
        Stroke::new(1.0, t.border.gamma_multiply(0.5)),
    );
    let mut x = line.left() + 6.0;
    if let Some(i) = icon {
        p.text(
            pos2(x + 9.0, line.center().y),
            Align2::CENTER_CENTER,
            i,
            theme::regular(17.0),
            t.text2,
        );
        x += 30.0;
    }
    p.text(
        pos2(x, line.center().y),
        Align2::LEFT_CENTER,
        what,
        theme::regular(14.0),
        t.text,
    );
    line
}

/// Keys as keycaps ending at `right`, centred on `cy`: rounded, with a
/// deeper bottom edge, like a key. Ctrl is Cmd on macOS. Returns where
/// they start.
fn keycaps(ui: &Ui, t: &Theme, right: f32, cy: f32, keys: &str, style: Cap) -> f32 {
    let p = ui.painter();
    let mut right = right;
    for k in keys.split(' ').filter(|k| !k.is_empty()).rev() {
        let k = if cfg!(target_os = "macos") && k == "Ctrl" {
            "Cmd"
        } else {
            k
        };
        let (fill, edge, ink) = match style {
            Cap::Plain => (t.hover, t.border, t.text),
            Cap::Clash => (t.hover, t.warn, t.text),
            Cap::Lit => (t.accent_soft(), t.accent, t.accent),
        };
        let g = p.layout_no_wrap(tr(k).to_string(), theme::medium(13.5), ink);
        let w = (g.size().x + 18.0).max(CAP_H);
        let cap = Rect::from_min_size(pos2(right - w, cy - CAP_H / 2.0), vec2(w, CAP_H));
        // The key's side, a little below its face.
        p.rect_filled(
            cap.translate(vec2(0.0, 2.0)),
            CornerRadius::same(7),
            edge.gamma_multiply(if style == Cap::Plain { 0.9 } else { 0.55 }),
        );
        p.rect(
            cap,
            CornerRadius::same(7),
            fill,
            Stroke::new(if style == Cap::Clash { 1.5 } else { 1.0 }, edge),
            egui::StrokeKind::Inside,
        );
        p.galley(cap.center() - g.size() / 2.0 - vec2(0.0, 0.5), g, ink);
        right = cap.left() - CAP_GAP;
    }
    right + CAP_GAP
}

/// The slot waiting for a key: an accent-edged keycap that pulses, with
/// "Press keys" in it. Ends at `right`.
fn waiting_slot(ui: &Ui, t: &Theme, right: f32, cy: f32, pulse: f32) -> Rect {
    let p = ui.painter();
    let g = p.layout_no_wrap(tr("Press keys").to_string(), theme::medium(13.5), t.accent);
    let w = g.size().x + 26.0;
    let cap = Rect::from_min_size(pos2(right - w, cy - CAP_H / 2.0), vec2(w, CAP_H));
    p.rect_filled(
        cap.expand(3.0 * pulse),
        CornerRadius::same(9),
        motion::with_alpha(t.accent, 0.12 * pulse),
    );
    p.rect(
        cap,
        CornerRadius::same(7),
        t.accent_soft(),
        Stroke::new(1.5, motion::with_alpha(t.accent, 0.55 + 0.45 * pulse)),
        egui::StrokeKind::Inside,
    );
    p.galley(cap.center() - g.size() / 2.0 - vec2(0.0, 0.5), g, t.accent);
    cap
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
            texts.contains("Folders") && texts.contains("Scripts") && texts.contains("Snapshots"),
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

//! Settings → Export and import: moves watchlists, layouts, scripts and the
//! rest to another device, as a file or straight over the local network.
//! The work is in `opentern_core::{transfer, handoff}`; this is the page and
//! its flows. Importing restarts the app (see `transfer`'s module doc).

use crate::app::App;
use crate::icons::ph;
use crate::motion;
use crate::theme::{self, Theme};
use crate::widgets::{self, Kind};
use crossbeam_channel::{Receiver, bounded};
use egui::{Align2, Color32, CornerRadius, Rect, Sense, Stroke, Ui, pos2, vec2};
use opentern_core::handoff::{self, Find, Job, RecvError, RecvEvent, SendEvent};
use opentern_core::transfer::{self, Item, Manifest, Mode};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum View {
    Export,
    Import,
}

#[derive(Clone, Debug, PartialEq)]
enum SendPhase {
    /// The file is being made.
    Preparing,
    Waiting,
    Sending {
        peer: String,
        progress: f32,
    },
    Sent {
        peer: String,
    },
    Failed(String),
}

struct Sending {
    code: String,
    phase: SendPhase,
    job: Option<Job<SendEvent>>,
    prepared: Option<Receiver<Result<(), String>>>,
    addresses: Vec<std::net::SocketAddr>,
    /// A wrong try or a dropped connection, while still waiting.
    note: Option<&'static str>,
}

#[derive(Clone, Debug, PartialEq)]
enum CodePhase {
    Typing,
    Searching,
    Receiving { peer: String, progress: f32 },
}

struct CodeEntry {
    text: String,
    phase: CodePhase,
    job: Option<Job<RecvEvent>>,
    error: Option<RecvError>,
    /// The address box, once asked for.
    address: Option<String>,
    focus: bool,
}

struct Review {
    path: PathBuf,
    /// Where it came from: a device's or a file's name.
    source: String,
    manifest: Manifest,
    /// What's really in it, not what the manifest claims.
    seen: transfer::Inspection,
    picks: BTreeMap<Item, bool>,
    mode: Mode,
    staging: Option<Receiver<Result<(), String>>>,
    error: Option<String>,
}

enum Flow {
    Idle,
    /// A system file dialog is open.
    PickingSave(Receiver<Option<PathBuf>>),
    PickingOpen(Receiver<Option<PathBuf>>),
    /// The export file is being written.
    Saving(Receiver<Result<PathBuf, String>>),
    Send(Box<Sending>),
    Code(Box<CodeEntry>),
    Review(Box<Review>),
}

/// The way out when the system stands between two devices: both prompts
/// below never ask twice once they've been turned down.
#[derive(Clone, Copy, PartialEq, Debug)]
enum NetKind {
    None,
    /// A sender must accept a connection: Windows Firewall.
    Firewall,
    /// Both sides use the local network: macOS's privacy setting.
    LocalNetwork,
}

enum FirewallAsk {
    Idle,
    Asking(Receiver<Result<(), String>>),
    Allowed,
    Failed,
}

struct NetHelp {
    kind: NetKind,
    ask: FirewallAsk,
    /// Runs on its own thread; stubbed in tests.
    allow: fn() -> Result<(), String>,
}

impl NetHelp {
    fn here() -> NetHelp {
        let kind = if cfg!(windows) {
            NetKind::Firewall
        } else if cfg!(target_os = "macos") {
            NetKind::LocalNetwork
        } else {
            NetKind::None
        };
        NetHelp {
            kind,
            ask: FirewallAsk::Idle,
            allow: handoff::firewall::ask_to_allow,
        }
    }

    fn start_asking(&mut self, ctx: &egui::Context) {
        let (tx, rx) = bounded(1);
        let (allow, ctx) = (self.allow, ctx.clone());
        std::thread::spawn(move || {
            let _ = tx.send(allow());
            ctx.request_repaint();
        });
        self.ask = FirewallAsk::Asking(rx);
    }

    fn show(&mut self, ui: &mut Ui, t: &Theme) {
        if let FirewallAsk::Asking(rx) = &self.ask
            && let Ok(r) = rx.try_recv()
        {
            if let Err(e) = &r {
                crate::diag::crumb(format!("firewall rule: {e}"));
            }
            self.ask = if r.is_ok() {
                FirewallAsk::Allowed
            } else {
                FirewallAsk::Failed
            };
        }
        let press = |ui: &mut Ui, text: &str, tip: &str| {
            let rich = egui::RichText::new(text)
                .font(theme::regular(12.0))
                .color(t.text3)
                .underline();
            ui.add(egui::Label::new(rich).selectable(false).sense(egui::Sense::click()))
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .on_hover_text(tip)
                .clicked()
        };
        match (self.kind, &self.ask) {
            (NetKind::None, _) => {}
            (NetKind::LocalNetwork, _) => {
                let tip = "macOS asks once. If Opentern isn't allowed, turn it on here, then try again.";
                if press(ui, "Open Local Network settings", tip) {
                    crate::app::open_url(crate::app::MACOS_LOCAL_NETWORK);
                }
            }
            (NetKind::Firewall, FirewallAsk::Asking(_)) => line(ui, t, "Waiting for Windows"),
            (NetKind::Firewall, FirewallAsk::Allowed) => line(ui, t, "Allowed through Windows Firewall"),
            (NetKind::Firewall, ask) => {
                let failed = matches!(ask, FirewallAsk::Failed);
                let text = if failed {
                    "Windows didn't allow it. Try again"
                } else {
                    "Allow through Windows Firewall"
                };
                let tip = "Not connecting? Your other device has to reach this one. Windows will ask for permission.";
                if press(ui, text, tip) {
                    self.start_asking(ui.ctx());
                }
            }
        }
    }
}

pub struct Transfer {
    pub view: View,
    picks: BTreeMap<Item, bool>,
    counts: BTreeMap<Item, usize>,
    flow: Flow,
    /// The export just saved.
    saved: Option<PathBuf>,
    error: Option<String>,
    port: u16,
    device: String,
    dir: PathBuf,
    net: NetHelp,
}

impl Drop for Transfer {
    /// Settings closed: the working copies go (anything in flight has been
    /// told to stop by its own `Job`).
    fn drop(&mut self) {
        transfer::remove_working_files(&self.dir);
    }
}

fn all_on() -> BTreeMap<Item, bool> {
    Item::ALL.into_iter().map(|i| (i, true)).collect()
}

fn icon(item: Item) -> &'static str {
    match item {
        Item::Watchlists => ph::LIST_BULLETS,
        Item::Layouts => ph::LAYOUT,
        Item::Drawings => ph::PENCIL_SIMPLE_LINE,
        Item::Templates => ph::STACK,
        Item::Favorites => ph::STAR,
        Item::Alerts => ph::BELL,
        Item::Indices => ph::CHART_PIE_SLICE,
        Item::Paper => ph::WALLET,
        Item::Scripts => ph::CODE,
        Item::Screener => ph::FUNNEL,
        Item::Lab => ph::FLASK,
    }
}

fn recv_message(e: &RecvError) -> String {
    match e {
        RecvError::NotFound => {
            "Can't find your other device. Keep its code showing, and check that both are on the same network and no firewall is blocking Opentern.".into()
        }
        RecvError::WrongCode => "That code doesn't match. Check the other device and try again.".into(),
        RecvError::Incompatible => "The other device runs a different version of Opentern. Update both, then try again.".into(),
        RecvError::Dropped => "The connection dropped. Try again.".into(),
        RecvError::Other(m) => m.clone(),
    }
}

/// A plain sentence in the page's quiet style.
fn line(ui: &mut Ui, t: &Theme, text: impl Into<String>) {
    ui.label(
        egui::RichText::new(text.into())
            .font(theme::regular(12.5))
            .color(t.text2),
    );
}

impl Transfer {
    pub fn new(app: &mut App) -> Transfer {
        app.save_for_transfer();
        let dir = app.store.dir().to_path_buf();
        // Whatever a crash or a file still in use left behind.
        transfer::remove_working_files(&dir);
        Transfer {
            view: View::Export,
            picks: all_on(),
            counts: transfer::counts(&dir).unwrap_or_default(),
            flow: Flow::Idle,
            saved: None,
            error: None,
            port: handoff::PORT,
            device: handoff::device_name(),
            dir,
            net: NetHelp::here(),
        }
    }

    fn selected(&self) -> Vec<Item> {
        Item::ALL
            .into_iter()
            .filter(|i| self.picks.get(i).copied().unwrap_or(true))
            .collect()
    }

    fn refresh(&mut self, app: &mut App) {
        app.save_for_transfer();
        self.counts = transfer::counts(&self.dir).unwrap_or_default();
    }

    // ----------------------------------------------------------- the actions

    /// Asks where to save, then exports.
    fn start_export(&mut self, app: &mut App) {
        if crate::app::headless() {
            return;
        }
        self.saved = None;
        self.error = None;
        self.refresh(app);
        let name = format!(
            "opentern-{}.{}",
            jiff::Zoned::now().strftime("%Y-%m-%d"),
            transfer::EXTENSION
        );
        let (tx, rx) = bounded(1);
        // Made here (some platforms need the UI thread), awaited on another.
        let fut = rfd::AsyncFileDialog::new()
            .set_title("Export")
            .set_file_name(&name)
            .add_filter("Opentern export", &[transfer::EXTENSION])
            .save_file();
        let ctx = app.egui_ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(pollster::block_on(fut).map(|h| h.path().to_path_buf()));
            ctx.request_repaint();
        });
        self.flow = Flow::PickingSave(rx);
    }

    /// Writes the export to `path`, off the UI thread.
    pub(crate) fn export_to(&mut self, app: &App, path: PathBuf) {
        let path = if path.extension().is_some_and(|e| e == transfer::EXTENSION) {
            path
        } else {
            let mut s = path.into_os_string();
            s.push(".");
            s.push(transfer::EXTENSION);
            PathBuf::from(s)
        };
        let (tx, rx) = bounded(1);
        let (dir, items, ctx) = (self.dir.clone(), self.selected(), app.egui_ctx.clone());
        std::thread::spawn(move || {
            let r = transfer::export(&dir, &items, &path)
                .map(|_| path)
                .map_err(|e| format!("Couldn't export: {e}"));
            let _ = tx.send(r);
            ctx.request_repaint();
        });
        self.flow = Flow::Saving(rx);
    }

    /// Makes the file, then offers it under a new code.
    pub(crate) fn start_send(&mut self, app: &mut App, code: Option<String>) {
        self.saved = None;
        self.error = None;
        self.refresh(app);
        let code = code.unwrap_or_else(handoff::new_code);
        if code.is_empty() {
            self.error = Some("Couldn't make a code on this computer.".into());
            return;
        }
        let (tx, rx) = bounded(1);
        let (dir, items, ctx) = (self.dir.clone(), self.selected(), app.egui_ctx.clone());
        std::thread::spawn(move || {
            let out = transfer::outgoing_file(&dir);
            let r = transfer::export(&dir, &items, &out)
                .map(|_| ())
                .map_err(|e| format!("Couldn't get your data ready: {e}"));
            let _ = tx.send(r);
            ctx.request_repaint();
        });
        self.flow = Flow::Send(Box::new(Sending {
            code,
            phase: SendPhase::Preparing,
            job: None,
            prepared: Some(rx),
            addresses: Vec::new(),
            note: None,
        }));
    }

    fn stop_send(&mut self) {
        self.flow = Flow::Idle;
        let _ = std::fs::remove_file(transfer::outgoing_file(&self.dir));
    }

    fn start_open(&mut self, app: &App) {
        if crate::app::headless() {
            return;
        }
        self.error = None;
        let (tx, rx) = bounded(1);
        let fut = rfd::AsyncFileDialog::new()
            .set_title("Import")
            .add_filter("Opentern export", &[transfer::EXTENSION])
            .pick_file();
        let ctx = app.egui_ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(pollster::block_on(fut).map(|h| h.path().to_path_buf()));
            ctx.request_repaint();
        });
        self.flow = Flow::PickingOpen(rx);
    }

    /// Opens a file for review.
    pub(crate) fn open_file(&mut self, path: &Path) {
        let source = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        self.review(path, source);
    }

    fn review(&mut self, path: &Path, source: String) {
        self.view = View::Import;
        match transfer::read_manifest(path) {
            Ok(manifest) => {
                let picks = manifest.present().into_iter().map(|(i, _)| (i, true)).collect();
                self.error = None;
                self.flow = Flow::Review(Box::new(Review {
                    path: path.to_path_buf(),
                    source,
                    manifest,
                    seen: transfer::inspect(path),
                    picks,
                    mode: Mode::Merge,
                    staging: None,
                    error: None,
                }));
            }
            Err(e) => {
                self.error = Some(e.to_string());
                self.flow = Flow::Idle;
            }
        }
    }

    pub(crate) fn start_code(&mut self) {
        self.error = None;
        self.view = View::Import;
        self.flow = Flow::Code(Box::new(CodeEntry {
            text: String::new(),
            phase: CodePhase::Typing,
            job: None,
            error: None,
            address: None,
            focus: true,
        }));
    }

    /// Starts looking for the sender with the code typed so far.
    fn start_receive(c: &mut CodeEntry, dir: &Path, device: &str, port: u16, ctx: &egui::Context) {
        let find = match c.address.as_deref().map(str::trim).filter(|a| !a.is_empty()) {
            Some(a) => Find::Address(a.to_string()),
            None => Find::Search(port),
        };
        let ctx = ctx.clone();
        c.error = None;
        c.phase = CodePhase::Searching;
        c.job = Some(handoff::receive(
            c.text.clone(),
            find,
            transfer::incoming_file(dir),
            device.to_string(),
            move || ctx.request_repaint(),
        ));
    }

    /// Verifies the file, backs up what's here, stages the import.
    fn start_staging(r: &mut Review, dir: &Path, ctx: &egui::Context) {
        let items: Vec<Item> = r.picks.iter().filter(|(_, on)| **on).map(|(i, _)| *i).collect();
        let (tx, rx) = bounded(1);
        let (dir, path, mode, ctx) = (dir.to_path_buf(), r.path.clone(), r.mode, ctx.clone());
        std::thread::spawn(move || {
            let result = transfer::stage(&dir, &path, &items, mode).map_err(|e| e.to_string());
            let _ = tx.send(result);
            ctx.request_repaint();
        });
        r.error = None;
        r.staging = Some(rx);
    }

    // ------------------------------------------------------------- the frame

    /// Applies what's arrived from the worker threads.
    fn poll(&mut self, app: &mut App, ctx: &egui::Context) {
        let mut next = None;
        match &mut self.flow {
            Flow::Idle => {}
            Flow::PickingSave(rx) => match rx.try_recv() {
                Ok(Some(path)) => {
                    self.flow = Flow::Idle;
                    self.export_to(app, path);
                    return;
                }
                Ok(None) | Err(crossbeam_channel::TryRecvError::Disconnected) => next = Some(Flow::Idle),
                Err(_) => {}
            },
            Flow::PickingOpen(rx) => match rx.try_recv() {
                Ok(Some(path)) => {
                    self.flow = Flow::Idle;
                    self.open_file(&path);
                    return;
                }
                Ok(None) | Err(crossbeam_channel::TryRecvError::Disconnected) => next = Some(Flow::Idle),
                Err(_) => {}
            },
            Flow::Saving(rx) => match rx.try_recv() {
                Ok(Ok(path)) => {
                    self.saved = Some(path);
                    next = Some(Flow::Idle);
                }
                Ok(Err(e)) => {
                    self.error = Some(e);
                    next = Some(Flow::Idle);
                }
                Err(crossbeam_channel::TryRecvError::Disconnected) => {
                    self.error = Some("Couldn't export.".into());
                    next = Some(Flow::Idle);
                }
                Err(_) => {}
            },
            Flow::Send(s) => {
                if let Some(rx) = &s.prepared {
                    match rx.try_recv() {
                        Ok(Ok(())) => {
                            s.prepared = None;
                            let wake = ctx.clone();
                            s.job = Some(handoff::send(
                                transfer::outgoing_file(&self.dir),
                                s.code.clone(),
                                self.device.clone(),
                                self.port,
                                move || wake.request_repaint(),
                            ));
                        }
                        Ok(Err(e)) => {
                            s.prepared = None;
                            s.phase = SendPhase::Failed(e);
                        }
                        Err(crossbeam_channel::TryRecvError::Disconnected) => {
                            s.prepared = None;
                            s.phase = SendPhase::Failed("Couldn't get your data ready.".into());
                        }
                        Err(_) => {}
                    }
                }
                let events = s.job.as_ref().map(|j| j.poll()).unwrap_or_default();
                for e in events {
                    match e {
                        SendEvent::Listening { addresses } => {
                            s.addresses = addresses;
                            s.phase = SendPhase::Waiting;
                        }
                        SendEvent::WrongCode => s.note = Some("Someone entered the wrong code."),
                        SendEvent::Connected { peer } => {
                            s.note = None;
                            s.phase = SendPhase::Sending { peer, progress: 0.0 };
                        }
                        SendEvent::Progress(p) => {
                            if let SendPhase::Sending { progress, .. } = &mut s.phase {
                                *progress = p;
                            }
                        }
                        SendEvent::Done => {
                            let peer = match &s.phase {
                                SendPhase::Sending { peer, .. } => peer.clone(),
                                _ => "your other device".into(),
                            };
                            s.phase = SendPhase::Sent { peer };
                            s.job = None;
                            let _ = std::fs::remove_file(transfer::outgoing_file(&self.dir));
                        }
                        SendEvent::Dropped => {
                            s.note = Some("The connection dropped. The code still works.");
                            s.phase = SendPhase::Waiting;
                        }
                        SendEvent::Failed(m) => {
                            s.phase = SendPhase::Failed(m);
                            s.job = None;
                            let _ = std::fs::remove_file(transfer::outgoing_file(&self.dir));
                        }
                    }
                }
            }
            Flow::Code(c) => {
                let events = c.job.as_ref().map(|j| j.poll()).unwrap_or_default();
                for e in events {
                    match e {
                        RecvEvent::Connected { peer } => {
                            c.phase = CodePhase::Receiving { peer, progress: 0.0 };
                        }
                        RecvEvent::Progress(p) => {
                            if let CodePhase::Receiving { progress, .. } = &mut c.phase {
                                *progress = p;
                            }
                        }
                        RecvEvent::Done { path, peer } => {
                            self.flow = Flow::Idle;
                            self.review(&path, peer);
                            return;
                        }
                        RecvEvent::Failed(err) => {
                            if err == RecvError::WrongCode {
                                c.text.clear();
                            }
                            c.error = Some(err);
                            c.phase = CodePhase::Typing;
                            c.job = None;
                            c.focus = true;
                        }
                    }
                }
            }
            Flow::Review(r) => {
                if let Some(rx) = &r.staging {
                    match rx.try_recv() {
                        Ok(Ok(())) => {
                            r.staging = None;
                            app.updater.relaunch(ctx);
                        }
                        Ok(Err(e)) => {
                            r.staging = None;
                            r.error = Some(e);
                        }
                        Err(crossbeam_channel::TryRecvError::Disconnected) => {
                            r.staging = None;
                            r.error = Some("Couldn't get the import ready.".into());
                        }
                        Err(_) => {}
                    }
                }
            }
        }
        if let Some(f) = next {
            self.flow = f;
        }
    }

    pub fn page(&mut self, ui: &mut Ui, app: &mut App, t: &Theme) {
        self.poll(app, ui.ctx());
        // A file dropped anywhere on the page opens for review.
        if matches!(self.flow, Flow::Idle) {
            let dropped = ui
                .ctx()
                .input(|i| i.raw.dropped_files.first().map(|f| f.path().to_path_buf()));
            if let Some(path) = dropped {
                self.open_file(&path);
            }
        }
        match &self.flow {
            Flow::Send(_) => self.send_page(ui, app, t),
            Flow::Code(_) => self.code_page(ui, t),
            Flow::Review(_) => self.review_page(ui, app, t),
            _ => {
                let before = self.view;
                widgets::segmented(
                    ui,
                    t,
                    &mut self.view,
                    &[(View::Export, "Export"), (View::Import, "Import")],
                );
                if before != self.view {
                    self.saved = None;
                    self.error = None;
                }
                ui.add_space(18.0);
                match self.view {
                    View::Export => self.export_page(ui, app, t),
                    View::Import => self.import_page(ui, app, t),
                }
            }
        }
    }

    fn busy(&self) -> bool {
        !matches!(self.flow, Flow::Idle)
    }

    fn export_page(&mut self, ui: &mut Ui, app: &mut App, t: &Theme) {
        let rows: Vec<(Item, usize)> = Item::ALL
            .into_iter()
            .map(|i| (i, self.counts.get(&i).copied().unwrap_or(0)))
            .collect();
        // A short window scrolls the list, so the buttons stay in view.
        let footer = 118.0;
        let room = (ui.clip_rect().bottom() - ui.cursor().top() - footer).max(170.0);
        egui::ScrollArea::vertical()
            .id_salt("transfer-items")
            .max_height(room)
            .auto_shrink([false, true])
            .show(ui, |ui| pick_list(ui, t, &rows, &mut self.picks));
        ui.add_space(10.0);
        line(ui, t, "API keys and passwords aren't included.");
        let unsaved = app.editor.docs.iter().filter(|d| d.dirty()).count();
        if unsaved > 0 && self.picks.get(&Item::Scripts).copied().unwrap_or(true) {
            ui.add_space(4.0);
            line(
                ui,
                t,
                if unsaved == 1 {
                    "1 script has unsaved changes. Save it to include them.".to_string()
                } else {
                    format!("{unsaved} scripts have unsaved changes. Save them to include them.")
                },
            );
        }
        ui.add_space(14.0);
        let any = self.picks.values().any(|on| *on);
        let idle = !self.busy();
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            let saving = matches!(self.flow, Flow::Saving(_));
            let label = if saving { "Exporting" } else { "Export" };
            if widgets::button(ui, Kind::Primary, any && idle, Some(ph::DOWNLOAD_SIMPLE), label).clicked()
                && any
                && idle
            {
                self.start_export(app);
            }
            if widgets::button(
                ui,
                Kind::Secondary,
                any && idle,
                Some(ph::PAPER_PLANE_TILT),
                "Send to another device",
            )
            .clicked()
                && any
                && idle
            {
                self.start_send(app, None);
            }
        });
        if let Some(path) = self.saved.clone() {
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                ui.label(
                    egui::RichText::new(ph::CHECK_CIRCLE)
                        .font(theme::regular(16.0))
                        .color(t.accent),
                );
                line(ui, t, format!("Saved to {}", crate::settings::shorten_home(&path)));
                if widgets::secondary(ui, "Show in folder").clicked()
                    && let Some(parent) = path.parent()
                {
                    crate::app::open_path(&parent.join("."));
                }
            });
        }
        if let Some(e) = self.error.clone() {
            ui.add_space(12.0);
            line(ui, t, e);
        }
    }

    fn import_page(&mut self, ui: &mut Ui, app: &App, t: &Theme) {
        let idle = !self.busy();
        let gap = 12.0;
        let w = ui.available_width();
        // Side by side when they fit, stacked in a narrow pane.
        let wide = w >= 460.0;
        let card = vec2(if wide { (w - gap) / 2.0 } else { w }, 158.0);
        let mut open = false;
        let mut receive = false;
        let draw = |ui: &mut Ui, glyph: &str, title: &str, button: &str, kind: Kind, hit: &mut bool| {
            let (r, _) = ui.allocate_exact_size(card, Sense::hover());
            let p = ui.painter();
            p.rect(
                r,
                CornerRadius::same(12),
                t.card,
                Stroke::new(1.0, t.border.gamma_multiply(0.7)),
                egui::StrokeKind::Inside,
            );
            p.text(
                pos2(r.center().x, r.top() + 42.0),
                Align2::CENTER_CENTER,
                glyph,
                theme::regular(30.0),
                t.text2,
            );
            p.text(
                pos2(r.center().x, r.top() + 78.0),
                Align2::CENTER_CENTER,
                title,
                theme::medium(14.0),
                t.text,
            );
            let mut c = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(Rect::from_min_max(pos2(r.left(), r.top() + 100.0), r.max))
                    .layout(egui::Layout::top_down(egui::Align::Center)),
            );
            if widgets::button(&mut c, kind, idle, None, button).clicked() && idle {
                *hit = true;
            }
        };
        if wide {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                draw(
                    ui,
                    ph::FILE_ARROW_UP,
                    "From a file",
                    "Choose file",
                    Kind::Secondary,
                    &mut open,
                );
                draw(
                    ui,
                    ph::WIFI_HIGH,
                    "From another device",
                    "Receive",
                    Kind::Secondary,
                    &mut receive,
                );
            });
        } else {
            draw(
                ui,
                ph::FILE_ARROW_UP,
                "From a file",
                "Choose file",
                Kind::Secondary,
                &mut open,
            );
            ui.add_space(gap);
            draw(
                ui,
                ph::WIFI_HIGH,
                "From another device",
                "Receive",
                Kind::Secondary,
                &mut receive,
            );
        }
        if open {
            self.start_open(app);
        }
        if receive {
            self.start_code();
        }
        if let Some(e) = self.error.clone() {
            ui.add_space(14.0);
            line(ui, t, e);
        }
    }

    fn send_page(&mut self, ui: &mut Ui, app: &App, t: &Theme) {
        let device = self.device.clone();
        let Flow::Send(s) = &mut self.flow else { return };
        let mut stop = false;
        let mut again = false;
        ui.vertical_centered(|ui| {
            ui.add_space(14.0);
            match &s.phase {
                SendPhase::Preparing | SendPhase::Waiting => {
                    link(
                        ui,
                        t,
                        ("This device", ph::MONITOR),
                        ("Other device", ph::MONITOR),
                        Link::Waiting,
                    );
                    ui.add_space(18.0);
                    title(ui, t, "Enter this code on your other device");
                    ui.add_space(4.0);
                    line(ui, t, "Settings → Export and import → Import → Receive");
                    ui.add_space(18.0);
                    code_row(ui, t, &s.code, None, Some(app));
                    ui.add_space(16.0);
                    let status = s.note.unwrap_or("Waiting for the other device");
                    pulse_line(ui, t, status);
                    ui.add_space(18.0);
                    if widgets::secondary(ui, "Cancel").clicked() {
                        stop = true;
                    }
                    if let Some(addr) = s.addresses.first() {
                        ui.add_space(14.0);
                        let all = s.addresses.iter().map(|a| a.to_string()).collect::<Vec<_>>().join("\n");
                        ui.label(
                            egui::RichText::new(format!("{device} · {}", addr.ip()))
                                .font(theme::regular(12.0))
                                .color(t.text3),
                        )
                        .on_hover_text(all);
                    }
                    if self.net.kind != NetKind::None {
                        ui.add_space(8.0);
                        self.net.show(ui, t);
                    }
                }
                SendPhase::Sending { peer, progress } => {
                    link(
                        ui,
                        t,
                        ("This device", ph::MONITOR),
                        (peer, ph::MONITOR),
                        Link::Progress(*progress),
                    );
                    ui.add_space(18.0);
                    title(ui, t, &format!("Sending to {peer}"));
                    ui.add_space(6.0);
                    line(ui, t, format!("{:.0}%", progress * 100.0));
                    ui.add_space(18.0);
                    if widgets::secondary(ui, "Cancel").clicked() {
                        stop = true;
                    }
                }
                SendPhase::Sent { peer } => {
                    link(ui, t, ("This device", ph::MONITOR), (peer, ph::MONITOR), Link::Done);
                    ui.add_space(18.0);
                    title(ui, t, &format!("Sent to {peer}"));
                    ui.add_space(18.0);
                    if widgets::primary(ui, "Done").clicked() {
                        stop = true;
                    }
                }
                SendPhase::Failed(m) => {
                    link(
                        ui,
                        t,
                        ("This device", ph::MONITOR),
                        ("Other device", ph::MONITOR),
                        Link::Idle,
                    );
                    ui.add_space(18.0);
                    title(ui, t, m);
                    ui.add_space(18.0);
                    ui.horizontal(|ui| {
                        centered_buttons(ui, 170.0);
                        if widgets::primary(ui, "Start again").clicked() {
                            again = true;
                        }
                        if widgets::secondary(ui, "Close").clicked() {
                            stop = true;
                        }
                    });
                }
            }
        });
        if stop {
            self.stop_send();
        } else if again {
            // Nothing to save first: the selection hasn't changed.
            let code = handoff::new_code();
            let (tx, rx) = bounded(1);
            let (dir, items, ctx) = (self.dir.clone(), self.selected(), app.egui_ctx.clone());
            std::thread::spawn(move || {
                let out = transfer::outgoing_file(&dir);
                let r = transfer::export(&dir, &items, &out)
                    .map(|_| ())
                    .map_err(|e| format!("Couldn't get your data ready: {e}"));
                let _ = tx.send(r);
                ctx.request_repaint();
            });
            self.flow = Flow::Send(Box::new(Sending {
                code,
                phase: SendPhase::Preparing,
                job: None,
                prepared: Some(rx),
                addresses: Vec::new(),
                note: None,
            }));
        }
    }

    fn code_page(&mut self, ui: &mut Ui, t: &Theme) {
        let (dir, device, port) = (self.dir.clone(), self.device.clone(), self.port);
        let Flow::Code(c) = &mut self.flow else { return };
        let mut cancel = false;
        let ctx = ui.ctx().clone();
        ui.vertical_centered(|ui| {
            ui.add_space(14.0);
            match c.phase.clone() {
                CodePhase::Receiving { peer, progress } => {
                    link(
                        ui,
                        t,
                        (&peer, ph::MONITOR),
                        ("This device", ph::MONITOR),
                        Link::Progress(progress),
                    );
                    ui.add_space(18.0);
                    title(ui, t, &format!("Receiving from {peer}"));
                    ui.add_space(6.0);
                    line(ui, t, format!("{:.0}%", progress * 100.0));
                    ui.add_space(18.0);
                    if widgets::secondary(ui, "Cancel").clicked() {
                        cancel = true;
                    }
                }
                phase => {
                    let searching = phase == CodePhase::Searching;
                    let look = if searching { Link::Waiting } else { Link::Idle };
                    link(ui, t, ("Other device", ph::MONITOR), ("This device", ph::MONITOR), look);
                    ui.add_space(18.0);
                    title(ui, t, "Enter the code from your other device");
                    ui.add_space(18.0);
                    code_input(ui, t, c, !searching);
                    ui.add_space(16.0);
                    if searching {
                        pulse_line(ui, t, "Looking for your other device");
                    } else if let Some(e) = &c.error {
                        // Wraps within a comfortable measure.
                        let m = recv_message(e);
                        let w = ui.available_width().min(440.0);
                        ui.allocate_ui(vec2(w, 0.0), |ui| {
                            ui.vertical_centered(|ui| line(ui, t, m));
                        });
                        // A receiver needs no way in, only macOS's permission.
                        if *e == RecvError::NotFound && self.net.kind == NetKind::LocalNetwork {
                            ui.add_space(8.0);
                            self.net.show(ui, t);
                        }
                    }
                    if let Some(a) = &mut c.address {
                        ui.add_space(12.0);
                        ui.add(
                            widgets::field(a)
                                .hint_text("Address, like 192.168.1.20")
                                .desired_width(240.0),
                        );
                    }
                    ui.add_space(18.0);
                    let complete = c.text.len() == handoff::CODE_LEN;
                    let can_retry = !searching && complete && c.error.is_some();
                    let can_address = !searching && c.address.is_none() && c.error == Some(RecvError::NotFound);
                    let width = 80.0 + if can_retry { 96.0 } else { 0.0 } + if can_address { 130.0 } else { 0.0 };
                    ui.horizontal(|ui| {
                        centered_buttons(ui, width);
                        if can_retry && widgets::primary(ui, "Try again").clicked() {
                            Transfer::start_receive(c, &dir, &device, port, &ctx);
                        }
                        if can_address && widgets::secondary(ui, "Enter address").clicked() {
                            c.address = Some(String::new());
                        }
                        if widgets::secondary(ui, "Cancel").clicked() {
                            cancel = true;
                        }
                    });
                }
            }
        });
        // Six digits: off it goes, unless this code already failed.
        if let Flow::Code(c) = &mut self.flow
            && c.phase == CodePhase::Typing
            && c.job.is_none()
            && c.error.is_none()
            && c.text.len() == handoff::CODE_LEN
        {
            Transfer::start_receive(c, &dir, &device, port, &ctx);
        }
        if cancel {
            self.flow = Flow::Idle;
            let _ = std::fs::remove_file(transfer::incoming_file(&self.dir));
        }
    }

    fn review_page(&mut self, ui: &mut Ui, app: &mut App, t: &Theme) {
        let dir = self.dir.clone();
        let Flow::Review(r) = &mut self.flow else { return };
        let mut cancel = false;
        ui.label(
            egui::RichText::new(format!("Import from {}", r.source))
                .font(theme::semibold(15.0))
                .color(t.text),
        );
        ui.add_space(2.0);
        let when = jiff::Timestamp::from_millisecond(r.manifest.created)
            .map(|ts| {
                ts.to_zoned(jiff::tz::TimeZone::system())
                    .strftime("%-d %b %Y, %H:%M")
                    .to_string()
            })
            .unwrap_or_default();
        line(ui, t, format!("Exported {when} · Opentern {}", r.manifest.app));
        ui.add_space(14.0);
        let rows = r.manifest.present();
        if rows.is_empty() {
            line(ui, t, "This file has nothing in it to import.");
        } else {
            pick_list(ui, t, &rows, &mut r.picks);
            ui.add_space(16.0);
            widgets::segmented(
                ui,
                t,
                &mut r.mode,
                &[(Mode::Merge, "Merge"), (Mode::Replace, "Replace")],
            );
            ui.add_space(8.0);
            line(
                ui,
                t,
                match r.mode {
                    Mode::Merge => {
                        "Keeps what's here and updates anything with the same name. A backup is saved first."
                    }
                    Mode::Replace => "Removes what's here for the items ticked above. A backup is saved first.",
                },
            );
            let warn = |ui: &mut Ui, text: String| {
                ui.add_space(10.0);
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    ui.label(
                        egui::RichText::new(ph::WARNING)
                            .font(theme::regular(14.0))
                            .color(t.warn),
                    );
                    line(ui, t, text);
                });
            };
            let on = |item: Item| r.picks.get(&item).copied().unwrap_or(false);
            if r.seen.python > 0 && on(Item::Scripts) {
                let what = if r.seen.python == 1 {
                    "This file has a Python script".to_string()
                } else {
                    format!("This file has {} Python scripts", r.seen.python)
                };
                warn(
                    ui,
                    format!(
                        "{what}, and Python has full access to this computer. Nothing will run until you press Run in the Script Editor."
                    ),
                );
            }
            if !r.seen.webhook_hosts.is_empty() && on(Item::Alerts) {
                warn(
                    ui,
                    format!("Alerts in this file send to {}.", r.seen.webhook_hosts.join(", ")),
                );
            }
        }
        if let Some(e) = &r.error {
            ui.add_space(10.0);
            line(ui, t, e.clone());
        }
        // Another window would keep the old data open across the import.
        let other_window = app.app_lock.is_none() && !crate::app::headless();
        if other_window {
            ui.add_space(10.0);
            line(ui, t, "Opentern is open in another window. Close it, then import.");
        }
        ui.add_space(16.0);
        let staging = r.staging.is_some();
        let any = r.picks.values().any(|on| *on);
        let can = any && !staging && !other_window;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            let (kind, label) = match (r.mode, staging) {
                (_, true) => (Kind::Primary, "Importing"),
                (Mode::Merge, _) => (Kind::Primary, "Import and restart"),
                (Mode::Replace, _) => (Kind::Danger, "Replace and restart"),
            };
            if widgets::button(ui, kind, can, None, label).clicked() && can {
                Transfer::start_staging(r, &dir, ui.ctx());
            }
            if widgets::button(ui, Kind::Secondary, !staging, None, "Cancel").clicked() && !staging {
                cancel = true;
            }
        });
        if cancel {
            self.flow = Flow::Idle;
            let _ = std::fs::remove_file(transfer::incoming_file(&self.dir));
        }
    }
}

impl App {
    /// Saves what's only in memory, so what's exported is what's on screen.
    pub(crate) fn save_for_transfer(&mut self) {
        self.save_layout_now();
        self.each_chart(|a| a.save_drawings(true));
        self.save_watchlists();
        self.save_paper_now();
        self.refresh_alert_context();
    }

    /// What the last import did (once, at launch).
    pub(crate) fn import_outcome(&mut self) {
        // Left by a run that ended mid-transfer.
        transfer::remove_working_files(self.store.dir());
        let Some(json) = self.store.value(transfer::RESULT_KEY).filter(|v| !v.is_empty()) else {
            return;
        };
        let _ = self.store.set_value(transfer::RESULT_KEY, "");
        let Ok(o) = serde_json::from_str::<transfer::Outcome>(&json) else {
            return;
        };
        let backups = o.backup.parent().map(|d| d.join("."));
        match o.error {
            None => self.notify(
                ph::CHECK_CIRCLE,
                "Import finished",
                "What was here is saved in the backups folder.",
                backups,
                true,
            ),
            Some(e) => self.notify(ph::WARNING, "Couldn't import", &e, backups, true),
        }
    }

    /// `OPENTERN_STILL=transfer:<what>`, with `settings:transfer`, for
    /// screenshots and trying the flow between two windows.
    pub(crate) fn transfer_still(&mut self, what: &str) {
        let mut page = self
            .settings
            .take()
            .unwrap_or_else(|| crate::settings::SettingsPage::new(crate::settings::Cat::Transfer));
        page.cat = crate::settings::Cat::Transfer;
        let mut x = page.transfer.take().unwrap_or_else(|| Transfer::new(self));
        let (head, arg) = what.split_once(':').unwrap_or((what, ""));
        let arg = arg.to_string();
        let progress = arg.parse::<f32>().unwrap_or(62.0) / 100.0;
        let peer = "Laptop".to_string();
        // How another system's page looks, from this one.
        let (head, net) = match head.split_once('@') {
            Some((h, "windows")) => (h, Some(NetKind::Firewall)),
            Some((h, "mac")) => (h, Some(NetKind::LocalNetwork)),
            _ => (head, None),
        };
        if let Some(kind) = net {
            x.net.kind = kind;
        }
        match head {
            "import" => x.view = View::Import,
            "saved" => x.saved = Some(self.store.dir().join("opentern-export.opentern")),
            "send" => x.start_send(self, Some(if arg.is_empty() { "482193".into() } else { arg })),
            // The code's copy button as it looks once pressed.
            "copied" => {
                x.start_send(self, Some("482193".into()));
                self.egui_ctx.data_mut(|d| d.insert_temp(copied_id(), 1e9_f64));
            }
            "code" | "receive" => {
                x.start_code();
                if let Flow::Code(c) = &mut x.flow {
                    c.text = arg;
                }
            }
            "review" => x.open_file(Path::new(&arg)),
            // Trying the whole restart: review a file and import it, merged.
            "import-now" => {
                x.open_file(Path::new(&arg));
                let (dir, ctx) = (x.dir.clone(), self.egui_ctx.clone());
                if let Flow::Review(r) = &mut x.flow {
                    Transfer::start_staging(r, &dir, &ctx);
                }
            }
            "sending" | "sent" | "send-failed" => {
                let phase = match head {
                    "sending" => SendPhase::Sending { peer, progress },
                    "sent" => SendPhase::Sent { peer },
                    _ => SendPhase::Failed("Too many wrong codes. Start again.".into()),
                };
                x.flow = Flow::Send(Box::new(Sending {
                    code: "482193".into(),
                    phase,
                    job: None,
                    prepared: None,
                    addresses: Vec::new(),
                    note: None,
                }));
            }
            "receiving" | "notfound" | "wrong" => {
                x.start_code();
                if let Flow::Code(c) = &mut x.flow {
                    match head {
                        "receiving" => c.phase = CodePhase::Receiving { peer, progress },
                        "notfound" => {
                            c.text = "482193".into();
                            c.error = Some(RecvError::NotFound);
                        }
                        _ => c.error = Some(RecvError::WrongCode),
                    }
                }
            }
            _ => {}
        }
        page.transfer = Some(x);
        self.settings = Some(page);
    }
}

// --------------------------------------------------------------------- pieces

fn title(ui: &mut Ui, t: &Theme, text: &str) {
    ui.label(egui::RichText::new(text).font(theme::semibold(16.0)).color(t.text));
}

/// Pads a horizontal row so `width` of buttons sits in its middle.
fn centered_buttons(ui: &mut Ui, width: f32) {
    ui.spacing_mut().item_spacing.x = 8.0;
    ui.add_space(((ui.available_width() - width) / 2.0).max(0.0));
}

/// The items with their checkboxes: a card of two columns when there's room.
/// A whole row toggles, not only its box.
fn pick_list(ui: &mut Ui, t: &Theme, rows: &[(Item, usize)], picks: &mut BTreeMap<Item, bool>) {
    let all = rows.iter().all(|(i, _)| picks.get(i).copied().unwrap_or(true));
    let (head, _) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::hover());
    ui.painter().text(
        pos2(head.left(), head.center().y),
        Align2::LEFT_CENTER,
        "INCLUDE",
        theme::semibold(11.0),
        t.text3,
    );
    let label = if all { "Select none" } else { "Select all" };
    let g = ui
        .painter()
        .layout_no_wrap(label.to_string(), theme::medium(12.0), t.text2);
    let hit = Rect::from_min_max(pos2(head.right() - g.size().x - 8.0, head.top()), head.max);
    let resp = ui
        .interact(hit, ui.id().with("select-all"), Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    let col = if resp.hovered() { t.text } else { t.text2 };
    ui.painter().galley(
        pos2(head.right() - g.size().x, head.center().y - g.size().y / 2.0),
        g,
        col,
    );
    if resp.clicked() {
        for (i, _) in rows {
            picks.insert(*i, !all);
        }
    }
    ui.add_space(4.0);

    let w = ui.available_width();
    let cols = if w >= 500.0 { 2 } else { 1 };
    let row_h = 40.0;
    let n_rows = rows.len().div_ceil(cols);
    let (card, _) = ui.allocate_exact_size(vec2(w, n_rows as f32 * row_h + 8.0), Sense::hover());
    ui.painter().rect(
        card,
        CornerRadius::same(12),
        t.card,
        Stroke::new(1.0, t.border.gamma_multiply(0.7)),
        egui::StrokeKind::Inside,
    );
    let inner = card.shrink2(vec2(6.0, 4.0));
    let cell_w = inner.width() / cols as f32;
    for (k, (item, count)) in rows.iter().enumerate() {
        // Down the first column, then the second.
        let col = k / n_rows;
        let row = k % n_rows;
        let r = Rect::from_min_size(
            pos2(inner.left() + col as f32 * cell_w, inner.top() + row as f32 * row_h),
            vec2(cell_w, row_h),
        )
        .shrink2(vec2(2.0, 1.0));
        #[cfg(test)]
        ui.ctx()
            .data_mut(|d| d.insert_temp(egui::Id::new(("pick-rect", *item)), r));
        let id = ui.id().with(("pick", item.title()));
        let resp = ui
            .interact(r, id, Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        let hov = motion::toggle(ui.ctx(), id.with("h"), ui.rect_contains_pointer(r), motion::MICRO);
        let p = ui.painter();
        p.rect_filled(r, CornerRadius::same(8), motion::with_alpha(t.hover_wash(), hov));
        let on = picks.entry(*item).or_insert(true);
        let text_col = if *on { t.text } else { t.text3 };
        p.text(
            pos2(r.left() + 42.0, r.center().y),
            Align2::CENTER_CENTER,
            icon(*item),
            theme::regular(15.0),
            if *on { t.text2 } else { t.text3 },
        );
        p.text(
            pos2(r.left() + 58.0, r.center().y),
            Align2::LEFT_CENTER,
            item.title(),
            theme::regular(13.0),
            text_col,
        );
        if !item.is_setting() {
            p.text(
                pos2(r.right() - 10.0, r.center().y),
                Align2::RIGHT_CENTER,
                count.to_string(),
                theme::regular(12.5),
                t.text3,
            );
        }
        let mut c = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(Rect::from_min_size(
                    pos2(r.left() + 10.0, r.center().y - 12.0),
                    vec2(20.0, 24.0),
                ))
                .id_salt(("pick-box", item.title())),
        );
        let box_clicked = widgets::checkbox(&mut c, on, "").clicked();
        if resp.clicked() && !box_clicked {
            *on = !*on;
        }
    }
}

/// How the line between the two devices looks.
#[derive(Clone, Copy, PartialEq)]
enum Link {
    Idle,
    Waiting,
    Progress(f32),
    Done,
}

/// Two devices with dots between them: a pulse travels while waiting, the
/// dots fill with the transfer, a check when it's done.
fn link(ui: &mut Ui, t: &Theme, from: (&str, &str), to: (&str, &str), state: Link) {
    let w = ui.available_width().min(340.0);
    let (rect, _) = ui.allocate_exact_size(vec2(w, 92.0), Sense::hover());
    let p = ui.painter();
    let r = 26.0;
    let y = rect.top() + r + 1.0;
    let left = pos2(rect.left() + 46.0, y);
    let right = pos2(rect.right() - 46.0, y);
    // 0 → 1 as the transfer finishes: everything below eases on this.
    let done = motion::toggle(
        ui.ctx(),
        ui.id().with("link-done"),
        state == Link::Done,
        motion::EMPHASIS,
    );
    for ((label, glyph), c) in [(from, left), (to, right)] {
        p.circle(
            c,
            r,
            motion::lerp_color(t.card, t.accent_soft(), done),
            Stroke::new(1.0, motion::lerp_color(t.border, t.accent, done)),
        );
        p.text(
            c,
            Align2::CENTER_CENTER,
            glyph,
            theme::regular(24.0),
            motion::lerp_color(t.text2, t.accent, done),
        );
        // Long names are cut to the slot under their device.
        let g = p.layout_no_wrap(label.to_string(), theme::regular(12.0), t.text3);
        let slot = Rect::from_center_size(pos2(c.x, y + r + 16.0), vec2(92.0, 18.0));
        let at = pos2(
            (c.x - g.size().x / 2.0).max(slot.left()),
            slot.center().y - g.size().y / 2.0,
        );
        p.with_clip_rect(slot.intersect(p.clip_rect())).galley(at, g, t.text3);
    }
    let start = left.x + r + 10.0;
    let end = right.x - r - 10.0;
    let n = 13usize;
    let filled = match state {
        Link::Progress(f) => motion::tween(ui.ctx(), ui.id().with("link-progress"), f, motion::CHART),
        Link::Done => 1.0,
        _ => 0.0,
    };
    let time = ui.input(|i| i.time) as f32;
    let middle = pos2((start + end) / 2.0, y);
    for i in 0..n {
        let f = i as f32 / (n - 1) as f32;
        let c = pos2(start + (end - start) * f, y);
        // The dots around the middle clear out to make room for the check.
        let room = if (c.x - middle.x).abs() < CHECK_ROOM {
            1.0 - done
        } else {
            1.0
        };
        if room <= 0.01 {
            continue;
        }
        let (col, rad) = match state {
            Link::Idle => (t.border, 2.0),
            Link::Waiting => {
                // A soft pulse moving towards the receiver.
                let head = (time * 0.55).fract() * 1.4 - 0.2;
                let k = (1.0 - ((f - head).abs() / 0.18)).clamp(0.0, 1.0);
                (motion::lerp_color(t.border, t.accent, k), 2.0 + k * 1.2)
            }
            Link::Progress(_) | Link::Done => {
                let k = ((filled - f) * (n as f32 - 1.0) + 1.0).clamp(0.0, 1.0);
                (motion::lerp_color(t.border, t.accent, k), 2.0 + k * 0.6)
            }
        };
        p.circle_filled(c, rad * room, motion::with_alpha(col, room));
    }
    if done > 0.01 {
        // A soft halo, then the disc with a slight pop, then the tick draws in.
        p.circle_filled(middle, 18.0 * done, motion::with_alpha(t.accent_soft(), done));
        let pop = done + 0.14 * (done * std::f32::consts::PI).sin();
        p.circle_filled(middle, 12.5 * pop, t.accent);
        tick(p, middle, 15.0, ((done - 0.35) / 0.65).clamp(0.0, 1.0), Color32::WHITE);
    }
    if state == Link::Waiting {
        // Only while this shows: idle screens cost nothing.
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(33));
    }
}

/// How far from the middle dots give way to the finished check.
const CHECK_ROOM: f32 = 22.0;

/// A tick `size` wide, drawn up to `t` (0..1) along its stroke.
fn tick(p: &egui::Painter, c: egui::Pos2, size: f32, t: f32, color: Color32) {
    if t <= 0.0 {
        return;
    }
    let a = c + vec2(-0.46, 0.02) * size;
    let b = c + vec2(-0.14, 0.34) * size;
    let e = c + vec2(0.46, -0.32) * size;
    let (first, second) = ((b - a).length(), (e - b).length());
    let reach = (first + second) * t;
    let mut points = vec![a];
    if reach <= first {
        points.push(a + (b - a) * (reach / first));
    } else {
        points.push(b);
        points.push(b + (e - b) * ((reach - first) / second));
    }
    p.add(egui::Shape::line(points, Stroke::new(size * 0.135, color)));
}

/// How long the copy button stays a check.
const COPIED_FOR: f64 = 1.6;

fn copied_id() -> egui::Id {
    egui::Id::new("transfer-code-copied")
}

/// The copy button: when pressed, the two squares fade as a check draws
/// itself in, then it turns back.
fn copy_button(ui: &mut Ui, t: &Theme, code: &str, rect: Rect) {
    let resp = ui
        .interact(rect, ui.id().with("transfer-copy"), Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    #[cfg(test)]
    ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new("copy-rect"), rect));
    let now = ui.input(|i| i.time);
    if resp.clicked() {
        ui.ctx().copy_text(code.to_string());
        ui.ctx().data_mut(|d| d.insert_temp(copied_id(), now));
    }
    let at = ui.ctx().data(|d| d.get_temp::<f64>(copied_id()));
    let copied = at.is_some_and(|a| now - a < COPIED_FOR);
    if let Some(a) = at.filter(|_| copied) {
        let left = (COPIED_FOR - (now - a)).clamp(0.0, COPIED_FOR);
        ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(left));
    }
    let k = motion::toggle(ui.ctx(), resp.id.with("copied"), copied, motion::STANDARD);
    let hover = motion::toggle(ui.ctx(), resp.id.with("h"), resp.hovered(), motion::MICRO);
    let resp = resp.on_hover_text(if copied { "Copied" } else { "Copy code" });
    let p = ui.painter();
    let round = CornerRadius::same(7);
    p.rect_filled(resp.rect, round, motion::with_alpha(t.hover_wash(), hover * (1.0 - k)));
    p.rect_filled(resp.rect, round, motion::with_alpha(t.accent_soft(), k));
    if k < 0.99 {
        let c = motion::lerp_color(t.text2, t.text, hover);
        p.text(
            rect.center(),
            Align2::CENTER_CENTER,
            ph::COPY,
            theme::regular(19.0),
            motion::with_alpha(c, 1.0 - k),
        );
    }
    tick(p, rect.center(), 17.0, k, t.accent);
}

/// A status with a slowly breathing dot.
fn pulse_line(ui: &mut Ui, t: &Theme, text: &str) {
    let g = ui
        .painter()
        .layout_no_wrap(text.to_string(), theme::regular(12.5), t.text2);
    let (r, _) = ui.allocate_exact_size(vec2(g.size().x + 16.0, 20.0), Sense::hover());
    let k = ((ui.input(|i| i.time) as f32 * 2.4).sin() + 1.0) / 2.0;
    ui.painter().circle_filled(
        pos2(r.left() + 4.0, r.center().y),
        3.5,
        motion::with_alpha(t.accent, 0.35 + 0.65 * k),
    );
    ui.painter()
        .galley(pos2(r.left() + 16.0, r.center().y - g.size().y / 2.0), g, t.text2);
}

const BOX: egui::Vec2 = vec2(44.0, 54.0);
const BOX_GAP: f32 = 8.0;
const MID_GAP: f32 = 20.0;

fn code_width() -> f32 {
    BOX.x * 6.0 + BOX_GAP * 4.0 + MID_GAP
}

/// Paints the six boxes in `rect`, `digits` in them.
fn paint_code(ui: &Ui, t: &Theme, rect: Rect, digits: &str, active: Option<usize>) {
    let p = ui.painter();
    let chars: Vec<char> = digits.chars().collect();
    let mut x = rect.left();
    for i in 0..handoff::CODE_LEN {
        let b = Rect::from_min_size(pos2(x, rect.top()), BOX);
        let on = active == Some(i);
        p.rect(
            b,
            CornerRadius::same(10),
            t.card,
            Stroke::new(if on { 1.5 } else { 1.0 }, if on { t.accent } else { t.border }),
            egui::StrokeKind::Inside,
        );
        if let Some(c) = chars.get(i) {
            p.text(
                b.center(),
                Align2::CENTER_CENTER,
                c.to_string(),
                theme::semibold(24.0),
                t.text,
            );
        }
        x += BOX.x + if i == 2 { MID_GAP } else { BOX_GAP };
    }
}

/// The code to read out, with a copy button beside it.
fn code_row(ui: &mut Ui, t: &Theme, code: &str, active: Option<usize>, app: Option<&App>) {
    let copy = 34.0;
    let total = code_width() + 8.0 + copy;
    let (rect, _) = ui.allocate_exact_size(vec2(total + copy + 8.0, BOX.y), Sense::hover());
    // Centred on the boxes, with the button hanging to the right.
    let boxes = Rect::from_min_size(
        pos2(rect.center().x - code_width() / 2.0, rect.top()),
        vec2(code_width(), BOX.y),
    );
    paint_code(ui, t, boxes, code, active);
    if app.is_some() {
        let r = Rect::from_center_size(
            pos2(boxes.right() + 8.0 + copy / 2.0, boxes.center().y),
            vec2(copy, copy),
        );
        copy_button(ui, t, code, r);
    }
}

/// Six boxes you type into: an invisible text field over them, so typing,
/// paste and backspace all behave as usual. Only digits stay.
fn code_input(ui: &mut Ui, t: &Theme, c: &mut CodeEntry, editable: bool) {
    let (rect, _) = ui.allocate_exact_size(vec2(code_width(), BOX.y), Sense::hover());
    let id = ui.id().with("transfer-code");
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect));
    child.visuals_mut().text_cursor.stroke = Stroke::NONE;
    child.visuals_mut().selection.bg_fill = Color32::TRANSPARENT;
    child.visuals_mut().widgets.hovered.bg_stroke = Stroke::NONE;
    let edit = egui::TextEdit::singleline(&mut c.text)
        .id(id)
        .frame(egui::Frame::NONE)
        .text_color(Color32::TRANSPARENT)
        .desired_width(rect.width());
    let resp = child.add_enabled(editable, edit);
    if c.focus && editable {
        resp.request_focus();
        c.focus = false;
    }
    let before = c.text.clone();
    c.text.retain(|ch| ch.is_ascii_digit());
    c.text.truncate(handoff::CODE_LEN);
    // A change after a failure starts a fresh try.
    if c.text != before || resp.changed() {
        c.error = None;
    }
    let active = (editable && resp.has_focus()).then_some(c.text.len().min(handoff::CODE_LEN - 1));
    paint_code(ui, t, rect, &c.text, active);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::drive::Driver;
    use crate::settings::Cat;
    use std::sync::atomic::{AtomicU16, Ordering};

    static NEXT_PORT: AtomicU16 = AtomicU16::new(48700);

    fn with_page<R>(d: &mut Driver, f: impl FnOnce(&mut Transfer, &mut App) -> R) -> R {
        let mut page = d.app.settings.take().expect("settings open");
        let mut x = page.transfer.take().unwrap_or_else(|| Transfer::new(&mut d.app));
        let r = f(&mut x, &mut d.app);
        page.transfer = Some(x);
        d.app.settings = Some(page);
        r
    }

    /// Runs frames until `f` says so, on this device and `other`.
    fn until(d: &mut Driver, other: Option<&mut Driver>, what: &str, f: impl Fn(&Transfer) -> bool) {
        let start = std::time::Instant::now();
        let mut other = other;
        loop {
            d.frame(Vec::new());
            if let Some(o) = other.as_deref_mut() {
                o.frame(Vec::new());
            }
            if f(d.app.settings.as_ref().unwrap().transfer.as_ref().unwrap()) {
                return;
            }
            assert!(start.elapsed().as_secs() < 30, "timed out waiting for: {what}");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    fn seed(d: &mut Driver) {
        d.app.watchlists[0].name = "From the desktop".into();
        d.app.save_watchlists();
        d.app.layout_name = "Desktop layout".into();
        let dir = d.app.scripts_dir();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("mine.pine"),
            "//@version=6\nindicator(\"Mine\")\nplot(close)\n",
        )
        .unwrap();
    }

    fn open_page(d: &mut Driver) {
        d.app.open_settings(Cat::Transfer);
        d.frame(Vec::new());
        d.frame(Vec::new());
    }

    #[test]
    fn the_page_shows_every_item_ticked_with_counts() {
        let mut d = Driver::new("transfer-page");
        seed(&mut d);
        open_page(&mut d);
        let texts = d.texts().join("\n");
        for item in Item::ALL {
            assert!(texts.contains(item.title()), "{} missing in {texts}", item.title());
        }
        assert!(texts.contains("Send to another device"), "{texts}");
        assert!(texts.contains("Export"), "{texts}");
        with_page(&mut d, |x, _| {
            assert_eq!(x.selected().len(), Item::ALL.len(), "everything starts ticked");
            assert_eq!(x.counts[&Item::Scripts], 1);
            assert_eq!(x.counts[&Item::Watchlists], 1, "the starter list counts once saved");
            assert_eq!(x.counts[&Item::Layouts], 1, "the current layout counts once saved");
        });
    }

    #[test]
    fn export_file_import_file_end_to_end() {
        let mut from = Driver::new("transfer-file-from");
        seed(&mut from);
        open_page(&mut from);
        let out = from.dir.join("my-export"); // no extension: one is added
        with_page(&mut from, |x, app| {
            x.picks.insert(Item::Paper, false);
            x.export_to(app, out.clone());
        });
        until(&mut from, None, "the file to save", |x| x.saved.is_some());
        let file = out.with_extension(transfer::EXTENSION);
        assert!(file.exists());
        let m = transfer::read_manifest(&file).unwrap();
        assert!(!m.items.contains_key("paper"), "unticked stays out");
        assert!(m.items["scripts"] == 1);

        let mut to = Driver::new("transfer-file-to");
        open_page(&mut to);
        with_page(&mut to, |x, _| x.open_file(&file));
        to.frame(Vec::new());
        let texts = to.texts().join("\n");
        assert!(texts.contains("Import from my-export.opentern"), "{texts}");
        assert!(texts.contains("Import and restart"), "{texts}");
        assert!(texts.contains("Merge") && texts.contains("Replace"), "{texts}");
        assert!(!texts.contains("Paper accounts"), "only what's in the file: {texts}");

        // Replace asks more plainly.
        with_page(&mut to, |x, _| {
            if let Flow::Review(r) = &mut x.flow {
                r.mode = Mode::Replace;
            }
        });
        to.frame(Vec::new());
        assert!(to.texts().join("\n").contains("Replace and restart"));

        with_page(&mut to, |x, app| {
            let dir = x.dir.clone();
            let Flow::Review(r) = &mut x.flow else {
                panic!("no review")
            };
            Transfer::start_staging(r, &dir, &app.egui_ctx);
        });
        until(
            &mut to,
            None,
            "the import to be staged",
            |x| matches!(&x.flow, Flow::Review(r) if r.staging.is_none()),
        );
        assert!(to.app.updater.relaunching(), "the app restarts itself");
        let dir = to.dir.clone();
        assert!(transfer::pending(&dir));

        // What the next start does.
        transfer::apply_pending(&dir);
        let reopened = Driver::open(dir.clone());
        assert_eq!(reopened.app.watchlists[0].name, "From the desktop");
        assert_eq!(reopened.app.layout_name, "Desktop layout");
        assert!(reopened.app.scripts_dir().join("mine.pine").exists());
        assert_eq!(reopened.app.script_files(), ["mine.pine"]);
        assert_eq!(
            reopened.app.notes.list.first().map(|n| n.title.as_str()),
            Some("Import finished"),
            "the app reports it once"
        );
        assert!(reopened.app.onboarding.is_none());
        drop(reopened);
        let again = Driver::open(dir);
        assert!(again.app.notes.list.is_empty(), "and only once");
    }

    /// The whole device-to-device flow with two real apps over a socket.
    #[test]
    fn send_and_receive_end_to_end() {
        let port = NEXT_PORT.fetch_add(1, Ordering::Relaxed);
        let mut from = Driver::new("transfer-send-from");
        seed(&mut from);
        open_page(&mut from);
        with_page(&mut from, |x, app| {
            x.port = port;
            x.device = "desktop".into();
            x.start_send(app, Some("123456".into()));
        });
        until(
            &mut from,
            None,
            "the code to show",
            |x| matches!(&x.flow, Flow::Send(s) if s.phase == SendPhase::Waiting),
        );
        let texts = from.texts().join("\n");
        assert!(texts.contains("Enter this code on your other device"), "{texts}");
        for digit in "123456".chars() {
            assert!(texts.contains(digit), "{digit} not shown: {texts}");
        }

        let mut to = Driver::new("transfer-send-to");
        open_page(&mut to);
        with_page(&mut to, |x, _| {
            x.port = port;
            x.device = "laptop".into();
            x.start_code();
        });
        to.frame(Vec::new());
        assert!(to.texts().join("\n").contains("Enter the code from your other device"));

        // A wrong code first, typed: told so, and the sender says so too.
        // Letters are ignored.
        to.frame(vec![egui::Event::Text("6x5a4".into())]);
        to.frame(vec![egui::Event::Text("321".into())]);
        let start = std::time::Instant::now();
        loop {
            to.frame(Vec::new());
            from.frame(Vec::new());
            let t = to.app.settings.as_ref().unwrap().transfer.as_ref().unwrap();
            if matches!(&t.flow, Flow::Code(c) if c.error == Some(RecvError::WrongCode)) {
                break;
            }
            assert!(start.elapsed().as_secs() < 30);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        to.frame(Vec::new());
        let texts = to.texts().join("\n");
        assert!(texts.contains("That code doesn't match"), "{texts}");
        with_page(&mut to, |x, _| {
            if let Flow::Code(c) = &x.flow {
                assert!(c.text.is_empty(), "cleared for another go");
            }
        });
        let f = from.app.settings.as_ref().unwrap().transfer.as_ref().unwrap();
        assert!(matches!(&f.flow, Flow::Send(s) if s.note == Some("Someone entered the wrong code.")));

        // Now the right one, pasted with a space in it.
        to.frame(Vec::new());
        to.frame(Vec::new());
        to.frame(vec![egui::Event::Paste("123 456".into())]);
        until(&mut to, Some(&mut from), "the file to arrive", |x| {
            matches!(&x.flow, Flow::Review(_))
        });
        for _ in 0..200 {
            from.frame(Vec::new());
            let f = from.app.settings.as_ref().unwrap().transfer.as_ref().unwrap();
            if matches!(&f.flow, Flow::Send(s) if matches!(s.phase, SendPhase::Sent { .. })) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let texts = from.texts().join("\n");
        assert!(texts.contains("Sent to laptop"), "{texts}");
        assert!(!transfer::outgoing_file(&from.dir).exists(), "nothing left behind");
        let texts = to.texts().join("\n");
        assert!(texts.contains("Import from desktop"), "{texts}");
        with_page(&mut to, |x, app| {
            let dir = x.dir.clone();
            let Flow::Review(r) = &mut x.flow else { panic!() };
            Transfer::start_staging(r, &dir, &app.egui_ctx);
        });
        until(
            &mut to,
            None,
            "the import to be staged",
            |x| matches!(&x.flow, Flow::Review(r) if r.staging.is_none()),
        );
        let dir = to.dir.clone();
        transfer::apply_pending(&dir);
        let reopened = Driver::open(dir);
        // Merge: its own list stays, and the imported one is the one in use.
        let names: Vec<_> = reopened.app.watchlists.iter().map(|w| w.name.as_str()).collect();
        assert_eq!(names, ["Watchlist", "From the desktop"]);
        assert_eq!(reopened.app.active_list, 1);
        assert_eq!(reopened.app.layout_name, "Desktop layout");
        assert!(reopened.app.scripts_dir().join("mine.pine").exists());
    }

    #[test]
    fn a_click_anywhere_on_a_row_toggles_it_once() {
        let mut d = Driver::new("transfer-rows");
        open_page(&mut d);
        let rect = |d: &Driver, item: Item| {
            d.app
                .egui_ctx
                .data(|x| x.get_temp::<Rect>(egui::Id::new(("pick-rect", item))))
                .expect("row drawn")
        };
        let on = |d: &Driver, item: Item| d.app.settings.as_ref().unwrap().transfer.as_ref().unwrap().picks[&item];
        let r = rect(&d, Item::Paper);
        assert!(on(&d, Item::Paper));
        // On the row's text.
        d.click(pos2(r.left() + 90.0, r.center().y));
        d.frame(Vec::new());
        assert!(!on(&d, Item::Paper), "the row turned it off");
        // On the box itself.
        d.click(pos2(r.left() + 18.0, r.center().y));
        d.frame(Vec::new());
        assert!(on(&d, Item::Paper), "the box turned it on, exactly once");
        // Its neighbours didn't move.
        assert!(Item::ALL.iter().all(|i| on(&d, *i)));
    }

    #[test]
    fn the_copy_button_copies_and_turns_into_a_check_for_a_moment() {
        let port = NEXT_PORT.fetch_add(1, Ordering::Relaxed);
        let mut d = Driver::new("transfer-copy");
        open_page(&mut d);
        with_page(&mut d, |x, app| {
            x.port = port;
            x.start_send(app, Some("246810".into()));
        });
        until(
            &mut d,
            None,
            "the code to show",
            |x| matches!(&x.flow, Flow::Send(s) if s.phase == SendPhase::Waiting),
        );
        let copied = |d: &Driver| d.app.egui_ctx.data(|x| x.get_temp::<f64>(copied_id()));
        assert_eq!(copied(&d), None);
        let r = d
            .app
            .egui_ctx
            .data(|x| x.get_temp::<Rect>(egui::Id::new("copy-rect")))
            .expect("button drawn");
        d.click(r.center());
        d.frame(Vec::new());
        assert!(copied(&d).is_some(), "the press registered");
    }

    #[test]
    fn the_firewall_way_out_asks_once_and_says_how_it_went() {
        let mut d = Driver::new("transfer-firewall");
        open_page(&mut d);
        let port = NEXT_PORT.fetch_add(1, Ordering::Relaxed);
        with_page(&mut d, |x, app| {
            x.port = port;
            x.net.kind = NetKind::Firewall;
            x.net.allow = || Ok(());
            x.start_send(app, Some("482193".into()));
        });
        until(
            &mut d,
            None,
            "waiting",
            |x| matches!(&x.flow, Flow::Send(s) if s.phase == SendPhase::Waiting),
        );
        let ctx = d.app.egui_ctx.clone();
        with_page(&mut d, |x, _| x.net.start_asking(&ctx));
        until(&mut d, None, "allowed", |x| matches!(x.net.ask, FirewallAsk::Allowed));
        with_page(&mut d, |x, _| {
            x.net.allow = || Err("declined".into());
            x.net.ask = FirewallAsk::Idle;
            x.net.start_asking(&ctx);
        });
        until(&mut d, None, "declined", |x| matches!(x.net.ask, FirewallAsk::Failed));
    }

    #[test]
    fn a_file_that_isnt_ours_says_so_and_stays_on_the_page() {
        let mut d = Driver::new("transfer-bad-file");
        open_page(&mut d);
        let bad = d.dir.join("holiday.opentern");
        std::fs::write(&bad, "photos").unwrap();
        with_page(&mut d, |x, _| x.open_file(&bad));
        d.frame(Vec::new());
        let texts = d.texts().join("\n");
        assert!(texts.contains("That isn't an Opentern export file."), "{texts}");
        assert!(texts.contains("Choose file"), "{texts}");
    }

    #[test]
    fn closing_settings_stops_a_send() {
        let port = NEXT_PORT.fetch_add(1, Ordering::Relaxed);
        let mut d = Driver::new("transfer-close");
        open_page(&mut d);
        with_page(&mut d, |x, app| {
            x.port = port;
            x.start_send(app, Some("111222".into()));
        });
        until(
            &mut d,
            None,
            "the code to show",
            |x| matches!(&x.flow, Flow::Send(s) if s.phase == SendPhase::Waiting),
        );
        assert!(transfer::outgoing_file(&d.dir.clone()).exists());
        d.app.settings = None;
        assert!(!transfer::outgoing_file(&d.dir.clone()).exists(), "no copy left behind");
        let start = std::time::Instant::now();
        while std::net::TcpListener::bind(("0.0.0.0", port)).is_err() {
            assert!(start.elapsed().as_secs() < 3, "still listening");
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }

    #[test]
    fn the_scripts_list_matches_the_editors() {
        // `core` has its own walk (it can't see `Lang`): same files.
        let mut d = Driver::new("transfer-scripts");
        let root = d.app.scripts_dir();
        std::fs::create_dir_all(root.join("sub/deep")).unwrap();
        std::fs::create_dir_all(root.join("__pycache__")).unwrap();
        std::fs::create_dir_all(root.join(".git")).unwrap();
        for f in [
            "a.pine",
            "b.py",
            "c.luau",
            "d.lua",
            "e.js",
            "f.ts",
            "sub/g.pine",
            "sub/deep/h.py",
            "notes.txt",
            "data.csv",
            "__pycache__/x.py",
            ".git/hook.py",
            ".hidden.pine",
        ] {
            std::fs::write(root.join(f), "x").unwrap();
        }
        let editor = d.app.script_files();
        open_page(&mut d);
        with_page(&mut d, |x, _| {
            assert_eq!(x.counts[&Item::Scripts], editor.len(), "{editor:?}");
        });
        let out = d.dir.join("scripts-only.opentern");
        transfer::export(&d.dir, &[Item::Scripts], &out).unwrap();
        let to = tempdir_near(&d);
        transfer::apply(&to, &out, &[Item::Scripts], Mode::Merge).unwrap();
        let (mut got, _) = crate::scripting::scan_scripts(&to.join("scripts"));
        got.sort();
        let mut want = editor;
        want.sort();
        assert_eq!(got, want);
    }

    fn tempdir_near(d: &Driver) -> PathBuf {
        let p = d
            .dir
            .with_file_name(format!("{}-to", d.dir.file_name().unwrap().to_string_lossy()));
        let _ = std::fs::remove_dir_all(&p);
        p
    }
}

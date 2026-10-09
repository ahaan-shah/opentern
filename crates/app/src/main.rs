#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

/// A shortcut label with the platform's command key: `shortcut!("K")` is
/// "Ctrl K" on Linux and Windows and "Cmd K" on macOS.
#[cfg(target_os = "macos")]
#[allow(unused_macros)]
macro_rules! shortcut {
    ($k:literal) => {
        concat!("Cmd ", $k)
    };
}
#[cfg(not(target_os = "macos"))]
#[allow(unused_macros)]
macro_rules! shortcut {
    ($k:literal) => {
        concat!("Ctrl ", $k)
    };
}

mod alerts;
mod app;
mod chart;
mod diag;
mod dialogs;
mod drawbar;
mod editor;
mod format;
mod icons;
mod layouts;
mod motion;
mod objects;
mod paper;
mod pine_host;
mod scripting;
mod shell;
mod strategy;
mod theme;
mod updater;
mod watchlists;
mod widgets;

use opentern_core::Store;
use std::path::PathBuf;

const HELP: &str = "\
opentern — a fast, local-first trading terminal

USAGE:
    opentern [OPTIONS]

OPTIONS:
    --data-dir <PATH>   Use a different workspace folder
    --demo              Made-up prices, no network: for trying things out
    -V, --version       Print version
    -h, --help          Print this help
";

fn main() -> eframe::Result<()> {
    #[cfg(windows)]
    if std::env::args().len() > 1 {
        attach_console();
    }
    let mut args = std::env::args().skip(1);
    let mut data_dir: Option<PathBuf> = None;
    let mut demo = std::env::var_os("OPENTERN_DEMO").is_some();
    while let Some(a) = args.next() {
        match a.as_str() {
            "-h" | "--help" => {
                print_quietly(HELP);
                return Ok(());
            }
            "-V" | "--version" => {
                println!("opentern {}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            "--data-dir" => data_dir = args.next().map(PathBuf::from),
            "--demo" => demo = true,
            other => {
                eprintln!("opentern: unknown option {other} (try --help)");
                std::process::exit(2);
            }
        }
    }

    // A custom workspace keeps its window and UI state with it, so a test
    // instance never touches the real app's saved state.
    let separate = data_dir.is_some();
    let dir = data_dir.unwrap_or_else(opentern_core::data_dir);
    diag::init(&dir);
    let store = match Store::open(&dir) {
        Ok(s) => s,
        Err(e) => fatal(&format!(
            "Couldn't open your Opentern workspace in {}:\n{e}\n\nNothing in it has been changed.",
            dir.display()
        )),
    };
    // If a renderer fails to start and we retry with another, the first
    // attempt may already have taken the store; reopen it from disk then.
    let slot = std::rc::Rc::new(std::cell::RefCell::new(Some(store)));

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Opentern")
            .with_app_id("opentern")
            .with_inner_size([1440.0, 900.0])
            .with_min_inner_size([1000.0, 640.0]),
        multisampling: 0,
        // Don't block in eglSwapBuffers waiting for vblank: on Wayland a hidden
        // window (another workspace) never gets frame callbacks, so a vsync'd
        // swap can hang the UI thread until the compositor says "not
        // responding". The app paces its own frames instead (see App::ui).
        glow_options: eframe::egui_glow::GlowConfiguration {
            vsync: false,
            ..Default::default()
        },
        centered: true,
        persist_window: true,
        persistence_path: separate.then(|| dir.clone()),
        ..Default::default()
    };
    let run = |options: eframe::NativeOptions| {
        let slot = slot.clone();
        let dir = dir.clone();
        eframe::run_native(
            "opentern",
            options,
            Box::new(move |cc| {
                let store = match slot.borrow_mut().take() {
                    Some(s) => s,
                    None => Store::open(&dir)?,
                };
                Ok(Box::new(app::App::new(cc, store, demo)))
            }),
        )
    };
    let result = run_with_fallback(options, run);
    if let Err(e) = &result {
        fatal(&format!(
            "Opentern couldn't open its window:\n{e}\n\nThis usually means the graphics driver is missing or out of date."
        ));
    }
    result
}

/// Starts the UI. On Windows, OpenGL comes first (lightest), and if it can't
/// start (no usable driver: VMs, remote desktop, some older machines) we
/// retry with Direct3D 12, which always has a software fallback.
/// `OPENTERN_RENDERER=glow|wgpu` forces one.
#[cfg(windows)]
fn run_with_fallback(
    options: eframe::NativeOptions,
    run: impl Fn(eframe::NativeOptions) -> eframe::Result<()>,
) -> eframe::Result<()> {
    use eframe::Renderer;
    let wgpu_options = |mut o: eframe::NativeOptions| {
        o.renderer = Renderer::Wgpu;
        o.wgpu_options.surface.present_mode = eframe::wgpu::PresentMode::AutoNoVsync;
        o
    };
    match std::env::var("OPENTERN_RENDERER").ok().as_deref() {
        Some("wgpu") => return run(wgpu_options(options)),
        Some("glow") => {
            let mut o = options;
            o.renderer = Renderer::Glow;
            return run(o);
        }
        _ => {}
    }
    let mut glow = options.clone();
    glow.renderer = Renderer::Glow;
    match run(glow) {
        Ok(()) => Ok(()),
        Err(e) => {
            diag::note(
                "RENDERER",
                &format!("OpenGL failed to start ({e}); retrying with Direct3D"),
            );
            run(wgpu_options(options))
        }
    }
}

#[cfg(not(windows))]
fn run_with_fallback(
    options: eframe::NativeOptions,
    run: impl Fn(eframe::NativeOptions) -> eframe::Result<()>,
) -> eframe::Result<()> {
    run(options)
}

/// Prints to stdout, ignoring a closed pipe (`opentern --help | head`).
fn print_quietly(text: &str) {
    use std::io::Write;
    let _ = std::io::stdout().lock().write_all(text.as_bytes());
}

/// Reports an error that stops Opentern from starting, in a dialog as well
/// as on stderr (a Windows app has no console to print to), then exits.
fn fatal(msg: &str) -> ! {
    diag::note("COULDN'T START", msg);
    if !app::headless() {
        let log = diag::log_path()
            .map(|p| format!("\n\nDetails are in {}", p.display()))
            .unwrap_or_default();
        let _ = rfd::MessageDialog::new()
            .set_level(rfd::MessageLevel::Error)
            .set_title("Opentern couldn't start")
            .set_description(format!("{msg}{log}"))
            .set_buttons(rfd::MessageButtons::Ok)
            .show();
    }
    std::process::exit(1);
}

/// A Windows GUI app has no console, so `opentern --version` from a terminal
/// would print nothing. Attach to the terminal it was started from, if any.
#[cfg(windows)]
fn attach_console() {
    // SAFETY: plain Win32 call with no pointers; failing (no parent console)
    // is harmless.
    unsafe {
        windows_sys::Win32::System::Console::AttachConsole(windows_sys::Win32::System::Console::ATTACH_PARENT_PROCESS);
    }
}

#[cfg(test)]
mod drive;

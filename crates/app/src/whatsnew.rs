//! "What's new" after an update: a card with the release's changes in a
//! few plain points, opened on the first launch of a new version. Settings
//! → About opens it again. Adapted from Magpie's (there it hangs off a
//! sidebar row; Opentern has no sidebar, so it opens by itself).
//!
//! Every release adds its notes to [`RELEASES`] (see CLAUDE.md → Releasing).

use crate::icons::ph;
use crate::theme::{self, Theme};
use egui::{Align2, CornerRadius, Rect, Sense, Ui, pos2, vec2};
use opentern_core::Store;
use opentern_core::update::is_newer;
use std::collections::HashMap;

/// One change, said simply.
pub struct Item {
    pub icon: &'static str,
    pub title: &'static str,
    pub body: &'static str,
    /// A picture of the change, when words aren't enough: a PNG from
    /// `assets/whatsnew/`, about 2:1, via `include_bytes!`.
    pub image: Option<&'static [u8]>,
}

pub struct Notes {
    pub version: &'static str,
    pub items: &'static [Item],
}

/// Newest first. Keep each to 2–4 points in everyday words: what changed
/// and why it helps, not how it works.
pub const RELEASES: &[Notes] = &[
    Notes {
        version: "0.2.1",
        items: &[
            Item {
                icon: ph::WIFI_HIGH,
                title: "Crypto charts stay up to date",
                body: "Binance prices now load on networks that block Binance's main servers, such as \
                       Jio in India. Charts and watchlists had been stuck on older prices there.",
                image: None,
            },
            Item {
                icon: ph::FIRE,
                title: "Hotlists show the period",
                body: "The change column says what it covers: the last 24 hours for crypto, today for stocks.",
                image: None,
            },
        ],
    },
    Notes {
        version: "0.2.0",
        items: &[
            Item {
                icon: ph::SPARKLE,
                title: "The agent can use the whole app",
                body: "Ask Claude or Codex to open charts, draw, run the screener, tune a strategy in \
                   the Strategy Lab, paper trade or set alerts. It does what you'd do with clicks.",
                image: None,
            },
            Item {
                icon: ph::REWIND,
                title: "Learn from real examples",
                body: "Ask the agent to teach you a pattern: it finds one in the chart's history, marks \
                   it and walks you through it in bar replay.",
                image: None,
            },
            Item {
                icon: ph::WINDOWS_LOGO,
                title: "Fixed on Windows",
                body: "Opentern's icon shows in Start, search and Settings, and symbol logos appear again.",
                image: None,
            },
            Item {
                icon: ph::GIFT,
                title: "See what's new after each update",
                body: "When Opentern updates, this card shows what changed. Settings → About opens it again.",
                image: None,
            },
        ],
    },
];

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Set by the updater when it restarts Opentern into a new version.
pub const UPDATED_FROM_ENV: &str = "OPENTERN_UPDATED_FROM";

/// The settings key with the last version this workspace ran.
const SEEN_KEY: &str = "seen_version";

pub fn release_url(version: &str) -> String {
    format!("https://github.com/ahaan-shah/opentern/releases/tag/v{version}")
}

/// The notes to show on this launch: every release newer than the one this
/// workspace last ran, up to the running one. `updated_from` (from the
/// updater's restart) wins over the saved version. With neither, someone
/// already onboarded has updated from a version before this existed, so they
/// see this release's notes; a brand-new workspace sees nothing.
pub fn to_show(
    releases: &'static [Notes],
    seen: Option<&str>,
    current: &str,
    onboarded: bool,
    updated_from: Option<&str>,
) -> Vec<&'static Notes> {
    match updated_from.or(seen) {
        Some(from) => releases
            .iter()
            .filter(|n| is_newer(n.version, from) && !is_newer(n.version, current))
            .collect(),
        None if onboarded => releases.iter().filter(|n| n.version == current).collect(),
        None => Vec::new(),
    }
}

/// The card's notes and pictures.
#[derive(Default)]
pub struct Card {
    pub notes: Vec<&'static Notes>,
    images: HashMap<(usize, usize), egui::TextureHandle>,
}

impl Card {
    /// This version's notes (Settings → About).
    pub fn current() -> Card {
        Card {
            notes: RELEASES.iter().filter(|n| n.version == VERSION).collect(),
            images: HashMap::new(),
        }
    }
}

/// The card to open at launch, if there's news. Remembers this version
/// either way (the card shows once). Demo, still and headless runs start
/// as seen (`OPENTERN_STILL=whatsnew` opens the card for screenshots).
pub fn at_launch(store: &mut Store, demo: bool) -> Option<Card> {
    let still = std::env::var("OPENTERN_STILL").ok();
    let card = if demo || still.is_some() || crate::app::headless() {
        None
    } else {
        let updated_from = std::env::var(UPDATED_FROM_ENV).ok();
        let notes = to_show(
            RELEASES,
            store.value(SEEN_KEY).as_deref(),
            VERSION,
            store.value(crate::onboarding::DONE_KEY).is_some(),
            updated_from.as_deref(),
        );
        (!notes.is_empty()).then_some(Card {
            notes,
            images: HashMap::new(),
        })
    };
    if store.value(SEEN_KEY).as_deref() != Some(VERSION) {
        let _ = store.set_value(SEEN_KEY, VERSION);
    }
    card.filter(|c| !c.notes.is_empty())
}

/// The card. Returns true when it should close.
pub fn card(c: &mut Card, ui: &mut Ui, t: &Theme) -> bool {
    let ctx = ui.ctx().clone();
    let mut close = false;
    ui.set_width(460.0);
    // The newest release being described (normally the running one).
    let version = c.notes.first().map_or(VERSION, |n| n.version);
    // Header: a gift in a soft accent tile, then the title.
    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(vec2(46.0, 46.0), Sense::hover());
        let p = ui.painter();
        p.rect_filled(r, CornerRadius::same(14), t.accent_soft());
        p.text(
            r.center(),
            Align2::CENTER_CENTER,
            ph::GIFT,
            theme::regular(23.0),
            t.accent,
        );
        ui.add_space(6.0);
        ui.vertical(|ui| {
            ui.add_space(1.0);
            ui.label(
                egui::RichText::new("What's new")
                    .font(theme::display(23.0))
                    .color(t.text),
            );
            ui.label(
                egui::RichText::new(format!("Opentern {version}"))
                    .font(theme::regular(12.5))
                    .color(t.text3),
            );
        });
    });
    ui.add_space(18.0);

    let notes = c.notes.clone();
    let several = notes.len() > 1;
    let width = ui.available_width();
    egui::ScrollArea::vertical()
        .id_salt("whatsnew-scroll")
        .max_height(440.0)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            ui.set_width(width);
            for (ni, n) in notes.iter().enumerate() {
                if several {
                    if ni > 0 {
                        ui.add_space(6.0);
                    }
                    ui.label(
                        egui::RichText::new(format!("IN {}", n.version))
                            .font(theme::semibold(10.5))
                            .color(t.text3),
                    );
                    ui.add_space(8.0);
                }
                for (ii, item) in n.items.iter().enumerate() {
                    point(c, ui, t, (ni, ii), item, width);
                    ui.add_space(16.0);
                }
            }
        });

    ui.add_space(4.0);
    // A row, not a right-to-left layout (that takes all a modal's height).
    ui.horizontal(|ui| {
        if crate::widgets::button(
            ui,
            crate::widgets::Kind::Secondary,
            true,
            Some(ph::ARROW_SQUARE_OUT),
            "Full release notes",
        )
        .clicked()
        {
            crate::app::open_url(&release_url(version));
        }
        ui.add_space((ui.available_width() - 76.0).max(0.0));
        if crate::widgets::primary(ui, "Got it").clicked() {
            close = true;
        }
    });
    if ctx.input(|i| i.key_pressed(egui::Key::Enter)) {
        close = true;
    }
    close
}

/// One point: icon tile, a short title, a sentence, and maybe a picture.
fn point(c: &mut Card, ui: &mut Ui, t: &Theme, key: (usize, usize), item: &Item, width: f32) {
    let text_x = 50.0;
    ui.horizontal_top(|ui| {
        let (r, _) = ui.allocate_exact_size(vec2(36.0, 36.0), Sense::hover());
        let p = ui.painter();
        p.rect_filled(
            r,
            CornerRadius::same(11),
            t.tint(t.accent, if t.dark { 0.16 } else { 0.1 }),
        );
        p.text(
            r.center(),
            Align2::CENTER_CENTER,
            item.icon,
            theme::regular(17.0),
            t.accent,
        );
        ui.add_space(text_x - 36.0 - ui.spacing().item_spacing.x);
        ui.vertical(|ui| {
            ui.set_width(width - text_x);
            ui.add_space(1.0);
            ui.label(
                egui::RichText::new(item.title)
                    .font(theme::semibold(15.0))
                    .color(t.text),
            );
            ui.add_space(2.0);
            ui.label(egui::RichText::new(item.body).font(theme::regular(13.5)).color(t.text2));
        });
    });
    let Some(bytes) = item.image else { return };
    let tex = c
        .images
        .entry(key)
        .or_insert_with(|| {
            let color = decode_png(bytes).unwrap_or_default();
            ui.ctx().load_texture(
                format!("whatsnew-{}-{}", key.0, key.1),
                color,
                egui::TextureOptions::LINEAR,
            )
        })
        .clone();
    let [iw, ih] = tex.size();
    if iw == 0 || ih == 0 {
        return;
    }
    ui.add_space(10.0);
    let w_img = width - text_x;
    let size = vec2(w_img, w_img * ih as f32 / iw as f32);
    let (slot, _) = ui.allocate_exact_size(vec2(width, size.y), Sense::hover());
    let rect = Rect::from_min_size(pos2(slot.left() + text_x, slot.top()), size);
    egui::Image::new(egui::load::SizedTexture::from_handle(&tex))
        .corner_radius(CornerRadius::same(12))
        .paint_at(ui, rect);
    ui.painter().rect_stroke(
        rect,
        CornerRadius::same(12),
        egui::Stroke::new(1.0, t.border),
        egui::StrokeKind::Inside,
    );
}

/// An RGBA (or RGB) PNG as an image.
fn decode_png(bytes: &[u8]) -> Option<egui::ColorImage> {
    let mut dec = png::Decoder::new(std::io::Cursor::new(bytes));
    dec.set_transformations(png::Transformations::EXPAND);
    let mut reader = dec.read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buf).ok()?;
    let size = [info.width as usize, info.height as usize];
    let px = &buf[..info.buffer_size()];
    Some(match info.color_type {
        png::ColorType::Rgba => egui::ColorImage::from_rgba_unmultiplied(size, px),
        png::ColorType::Rgb => egui::ColorImage::from_rgb(size, px),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const FAKE: &[Notes] = &[
        Notes {
            version: "0.2.5",
            items: &[],
        },
        Notes {
            version: "0.2.4",
            items: &[],
        },
        Notes {
            version: "0.2.3",
            items: &[],
        },
    ];

    fn versions(v: Vec<&Notes>) -> Vec<&str> {
        v.iter().map(|n| n.version).collect()
    }

    #[test]
    fn after_an_update_shows_whats_newer() {
        assert_eq!(versions(to_show(FAKE, Some("0.2.3"), "0.2.4", true, None)), ["0.2.4"]);
        // Skipped a version: both, newest first.
        assert_eq!(
            versions(to_show(FAKE, Some("0.2.3"), "0.2.5", true, None)),
            ["0.2.5", "0.2.4"]
        );
        // The updater's restart says where it came from.
        assert_eq!(
            versions(to_show(FAKE, Some("0.2.4"), "0.2.4", true, Some("0.2.3"))),
            ["0.2.4"]
        );
    }

    #[test]
    fn nothing_without_an_update() {
        assert!(to_show(FAKE, Some("0.2.4"), "0.2.4", true, None).is_empty());
        // A new workspace: nothing to announce.
        assert!(to_show(FAKE, None, "0.2.4", false, None).is_empty());
        // Running an older build than the notes: never shows future notes.
        assert!(to_show(FAKE, Some("0.2.2"), "0.2.2", true, None).is_empty());
    }

    #[test]
    fn existing_users_from_before_this_feature_see_this_release() {
        assert_eq!(versions(to_show(FAKE, None, "0.2.4", true, None)), ["0.2.4"]);
    }

    /// Every release from 0.2.0 on must say what's new (CLAUDE.md →
    /// Releasing): bumping the version without notes fails here.
    #[test]
    fn this_release_has_notes() {
        if !is_newer("0.2.0", VERSION) {
            let notes = RELEASES.iter().find(|n| n.version == VERSION);
            assert!(
                notes.is_some_and(|n| !n.items.is_empty()),
                "add What's new notes for {VERSION} to whatsnew::RELEASES"
            );
        }
        // Newest first, so a skipped-version card reads top-down.
        assert!(RELEASES.windows(2).all(|w| is_newer(w[0].version, w[1].version)));
    }
}

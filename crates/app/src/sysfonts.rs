//! The computer's own fonts as fallbacks, for text the bundled fonts can't
//! draw: Chinese, Japanese and Korean in a script's labels, Arabic, Hebrew,
//! Thai, the Indic scripts and so on in names people type. The bundled
//! fonts (Inter and the translation subsets) stay first, so the app looks
//! the same; these only fill in characters nothing else has.
//!
//! Found once per run on a background thread (`fontdb`: the system font
//! folders, and fontconfig's on Linux). Each file is memory-mapped, not
//! read: the system pages in only what's drawn, so a 20 MB Chinese font
//! costs next to nothing for someone who never shows Chinese. A computer
//! without a font for a script still shows boxes for it, as before.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

/// One fallback face: its key in egui's font table, its bytes (a mapped
/// file, kept for the life of the app) and its index in a collection.
pub struct Face {
    pub key: String,
    pub bytes: &'static [u8],
    pub index: u32,
}

static FACES: OnceLock<Vec<Face>> = OnceLock::new();
static STARTED: AtomicBool = AtomicBool::new(false);
static NEW: AtomicBool = AtomicBool::new(false);

/// Families to look for, in order. Chinese, Japanese and Korean share
/// characters drawn differently; `cjk` puts the interface language's first.
fn wanted(cjk: &[&'static str]) -> Vec<&'static str> {
    let mut v: Vec<&'static str> = vec![
        // Broad coverage first: Latin, Greek and Cyrillic beyond Inter's,
        // and much else.
        "Noto Sans",
        "Segoe UI",
        "Segoe UI Historic",
        "DejaVu Sans",
        // Right-to-left.
        "Noto Sans Arabic",
        "Noto Naskh Arabic",
        "Geeza Pro",
        "Noto Sans Hebrew",
        "Arial Hebrew",
        // South and South-East Asia.
        "Nirmala UI",
        "Noto Sans Devanagari",
        "Noto Sans Bengali",
        "Noto Sans Tamil",
        "Noto Sans Telugu",
        "Noto Sans Kannada",
        "Noto Sans Malayalam",
        "Noto Sans Gujarati",
        "Noto Sans Gurmukhi",
        "Noto Sans Oriya",
        "Noto Sans Sinhala",
        "Kohinoor Devanagari",
        "Leelawadee UI",
        "Noto Sans Thai",
        "Thonburi",
        "Noto Sans Lao",
        "Noto Sans Khmer",
        "Myanmar Text",
        "Noto Sans Myanmar",
        // Elsewhere.
        "Noto Sans Georgian",
        "Noto Sans Armenian",
        "Noto Sans Ethiopic",
        "Ebrima",
        "Noto Sans Tibetan",
        "Microsoft Himalaya",
        "Mongolian Baiti",
    ];
    v.extend_from_slice(cjk);
    v.extend_from_slice(&[
        // Symbols and a last resort that covers a lot.
        "Noto Sans Symbols",
        "Noto Sans Symbols 2",
        "Segoe UI Symbol",
        "Apple Symbols",
        "Arial Unicode MS",
    ]);
    v
}

/// Chinese, Japanese and Korean families, the interface language's first.
fn cjk(lang: crate::i18n::Lang) -> Vec<&'static str> {
    let sc = [
        "Noto Sans CJK SC",
        "Source Han Sans SC",
        "Microsoft YaHei",
        "PingFang SC",
        "Hiragino Sans GB",
        "WenQuanYi Micro Hei",
        "WenQuanYi Zen Hei",
    ];
    let jp = [
        "Noto Sans CJK JP",
        "Source Han Sans",
        "Yu Gothic",
        "Meiryo",
        "Hiragino Sans",
    ];
    let kr = ["Noto Sans CJK KR", "Malgun Gothic", "Apple SD Gothic Neo"];
    let tc = ["Noto Sans CJK TC", "Microsoft JhengHei", "PingFang TC"];
    let mut v = Vec::new();
    if lang == crate::i18n::Lang::Japanese {
        v.extend(jp);
        v.extend(sc);
    } else {
        v.extend(sc);
        v.extend(jp);
    }
    v.extend(kr);
    v.extend(tc);
    v
}

/// Looks for the fallbacks once, on a background thread; `repaint` wakes
/// the window when they're ready (the fonts are then installed again).
pub fn start(repaint: impl Fn() + Send + 'static) {
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    let lang = crate::i18n::current();
    let _ = std::thread::Builder::new().name("system-fonts".into()).spawn(move || {
        let faces = find(&wanted(&cjk(lang)));
        if !faces.is_empty() {
            crate::diag::crumb(format!("{} system fallback fonts", faces.len()));
        }
        let _ = FACES.set(faces);
        NEW.store(true, Ordering::SeqCst);
        repaint();
    });
}

/// True once, after the fallbacks were found: time to install fonts again.
pub fn take_ready() -> bool {
    NEW.swap(false, Ordering::SeqCst)
}

/// The fallbacks found (none until the search is done, or in tests).
pub fn faces() -> &'static [Face] {
    FACES.get().map_or(&[], |v| v.as_slice())
}

/// The regular face of each family present, in order, mapped.
fn find(families: &[&str]) -> Vec<Face> {
    let mut db = fontdb::Database::new();
    db.load_system_fonts();
    let mut out: Vec<Face> = Vec::new();
    let mut seen: Vec<(std::path::PathBuf, u32)> = Vec::new();
    for name in families {
        let query = fontdb::Query {
            families: &[fontdb::Family::Name(name)],
            weight: fontdb::Weight::NORMAL,
            stretch: fontdb::Stretch::Normal,
            style: fontdb::Style::Normal,
        };
        let Some(id) = db.query(&query) else { continue };
        // The query falls back to another family when the one asked for is
        // missing; only the family itself is wanted.
        let Some(face) = db.face(id) else { continue };
        if !face.families.iter().any(|(f, _)| f.eq_ignore_ascii_case(name)) {
            continue;
        }
        let fontdb::Source::File(path) = &face.source else {
            continue;
        };
        if seen.iter().any(|(p, i)| p == path && *i == face.index) {
            continue;
        }
        let Some(bytes) = map(path) else { continue };
        seen.push((path.clone(), face.index));
        out.push(Face {
            key: format!("system:{name}"),
            bytes,
            index: face.index,
        });
    }
    out
}

/// A font file mapped for the rest of the run (never unmapped: egui keeps
/// using it). `None` if it can't be opened or isn't a font.
fn map(path: &std::path::Path) -> Option<&'static [u8]> {
    let file = std::fs::File::open(path).ok()?;
    // SAFETY: the mapping is read-only and lives for the rest of the run.
    // A font file rewritten in place while mapped could change under it;
    // package managers replace fonts with new files instead, which leaves
    // this mapping on the old one.
    let mapped = unsafe { memmap2::Mmap::map(&file) }.ok()?;
    let bytes: &'static [u8] = Box::leak(Box::new(mapped));
    // Only real fonts: TrueType, OpenType, a collection.
    matches!(bytes.get(..4), Some(b"\0\x01\0\0" | b"OTTO" | b"ttcf" | b"true")).then_some(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_interface_languages_cjk_comes_first() {
        assert_eq!(cjk(crate::i18n::Lang::Japanese)[0], "Noto Sans CJK JP");
        assert_eq!(cjk(crate::i18n::Lang::English)[0], "Noto Sans CJK SC");
        let all = wanted(&cjk(crate::i18n::Lang::English));
        assert!(all.iter().position(|f| *f == "Noto Sans CJK SC") < all.iter().position(|f| *f == "Arial Unicode MS"));
    }

    /// On this computer: whatever is found maps and is a font egui can use.
    #[test]
    #[ignore]
    fn finds_and_maps_this_computers_fallbacks() {
        let faces = find(&wanted(&cjk(crate::i18n::Lang::English)));
        for f in &faces {
            println!("{} ({} bytes, face {})", f.key, f.bytes.len(), f.index);
            let ctx = egui::Context::default();
            let mut fonts = egui::FontDefinitions::default();
            fonts.font_data.insert(
                f.key.clone(),
                std::sync::Arc::new(egui::FontData {
                    font: std::borrow::Cow::Borrowed(f.bytes),
                    index: f.index,
                    tweak: Default::default(),
                }),
            );
            fonts
                .families
                .get_mut(&egui::FontFamily::Proportional)
                .unwrap()
                .push(f.key.clone());
            ctx.set_fonts(fonts);
            let _ = ctx.run_ui(Default::default(), |ui| {
                ui.label("中文 日本語 한국어 العربية עברית ไทย हिन्दी");
            });
        }
    }
}

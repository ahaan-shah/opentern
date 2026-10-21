//! Files attached to a message: picked, dropped, pasted (copied files, or
//! an image on the clipboard). Claude gets images as images, text files
//! inline with their path (so it can still edit them), anything else as a
//! path to read.

use crate::icons::ph;
use base64::Engine;
use crossbeam_channel::Receiver;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

/// Images Claude sees directly.
const IMAGE: [(&str, &str); 5] = [
    ("png", "image/png"),
    ("jpg", "image/jpeg"),
    ("jpeg", "image/jpeg"),
    ("gif", "image/gif"),
    ("webp", "image/webp"),
];
/// Larger images go as a path (the API's limit is about 5 MB).
const MAX_IMAGE: u64 = 4_500_000;
/// Text files up to this size go inline.
const MAX_TEXT: u64 = 256 * 1024;

fn ext(p: &Path) -> String {
    p.extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
}

fn image_type(p: &Path) -> Option<&'static str> {
    let e = ext(p);
    IMAGE.iter().find(|(x, _)| *x == e).map(|(_, m)| *m)
}

/// Whether a file is an image the agents can look at.
pub fn is_image(p: &str) -> bool {
    image_type(Path::new(p)).is_some()
}

/// The icon for an attached file.
pub fn icon(p: &str) -> &'static str {
    let p = Path::new(p);
    if image_type(p).is_some() {
        return ph::IMAGE;
    }
    match ext(p).as_str() {
        "pdf" => ph::FILE_PDF,
        "pine" | "luau" | "lua" | "js" | "ts" | "py" | "rs" | "json" | "toml" | "yaml" | "yml" => ph::FILE_CODE,
        "csv" | "tsv" | "xlsx" | "xls" => ph::FILE_CSV,
        "txt" | "md" | "log" => ph::FILE_TEXT,
        _ => ph::FILE,
    }
}

/// The content blocks for these files, ahead of the message's text.
pub fn blocks(files: &[String]) -> Vec<Value> {
    let mut out = Vec::new();
    for f in files {
        let p = Path::new(f);
        let size = std::fs::metadata(p).map(|m| m.len()).unwrap_or(u64::MAX);
        if let Some(mime) = image_type(p).filter(|_| size <= MAX_IMAGE)
            && let Ok(bytes) = std::fs::read(p)
        {
            out.push(json!({"type": "text", "text": format!("Attached image: {f}")}));
            out.push(json!({"type": "image", "source": {
                "type": "base64",
                "media_type": mime,
                "data": base64::engine::general_purpose::STANDARD.encode(bytes),
            }}));
            continue;
        }
        if size <= MAX_TEXT
            && image_type(p).is_none()
            && let Ok(bytes) = std::fs::read(p)
            && let Ok(text) = String::from_utf8(bytes)
        {
            out.push(json!({"type": "text", "text": format!(
                "<attached_file path=\"{f}\">\n{text}\n</attached_file>"
            )}));
            continue;
        }
        out.push(json!({"type": "text", "text": format!(
            "Attached file (not shown inline; read it from its path if you need it): {f}"
        )}));
    }
    out
}

/// Files a paste names: `file://` URIs or absolute paths, one per line,
/// all existing. Anything else is ordinary text.
pub fn pasted_files(text: &str) -> Option<Vec<PathBuf>> {
    let lines: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .collect();
    if lines.is_empty() {
        return None;
    }
    let mut out = Vec::new();
    for l in lines {
        let path = match l.strip_prefix("file://") {
            Some(rest) => {
                // file://host/path: drop a host.
                let rest = if rest.starts_with('/') {
                    rest
                } else {
                    rest.find('/').map(|i| &rest[i..])?
                };
                PathBuf::from(
                    percent_encoding::percent_decode_str(rest)
                        .decode_utf8()
                        .ok()?
                        .into_owned(),
                )
            }
            None => PathBuf::from(l),
        };
        if !path.is_absolute() || !path.is_file() {
            return None;
        }
        out.push(path);
    }
    Some(out)
}

/// The clipboard's files, or its image saved as a PNG in `dir`.
pub fn from_clipboard(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let mut cb = arboard::Clipboard::new().map_err(|e| format!("Couldn't open the clipboard: {e}"))?;
    if let Ok(files) = cb.get().file_list()
        && !files.is_empty()
    {
        return Ok(files.into_iter().filter(|p| p.is_file()).collect());
    }
    let img = cb
        .get_image()
        .map_err(|_| "There's no image or file on the clipboard.".to_string())?;
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let path = dir.join(format!("pasted-{}.png", jiff::Timestamp::now().as_millisecond()));
    let file = std::fs::File::create(&path).map_err(|e| e.to_string())?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), img.width as u32, img.height as u32);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut w = enc.write_header().map_err(|e| e.to_string())?;
    w.write_image_data(&img.bytes).map_err(|e| e.to_string())?;
    Ok(vec![path])
}

/// Opens the system's file picker without blocking the UI; the files
/// arrive on the returned channel.
pub fn pick() -> Receiver<Vec<PathBuf>> {
    let (tx, rx) = crossbeam_channel::bounded(1);
    // Made here (some platforms need the UI thread), awaited on another.
    let fut = rfd::AsyncFileDialog::new().set_title("Attach files").pick_files();
    std::thread::spawn(move || {
        let files = pollster::block_on(fut)
            .unwrap_or_default()
            .into_iter()
            .map(|h| h.path().to_path_buf())
            .collect();
        let _ = tx.send(files);
    });
    rx
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_and_pastes() {
        let dir = std::env::temp_dir().join(format!("opentern-attach-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("my script.pine");
        std::fs::write(&script, "//@version=6\nindicator(\"X\")").unwrap();
        let img = dir.join("chart.png");
        std::fs::write(&img, [0x89, b'P', b'N', b'G']).unwrap();
        let bin = dir.join("data.bin");
        std::fs::write(&bin, [0u8, 159, 146, 150]).unwrap();
        let files: Vec<String> = [&script, &img, &bin]
            .iter()
            .map(|p| p.to_string_lossy().into())
            .collect();
        let b = blocks(&files);
        assert!(b[0]["text"].as_str().unwrap().contains("indicator(\"X\")"));
        assert_eq!(b[2]["type"], "image");
        assert_eq!(b[2]["source"]["media_type"], "image/png");
        assert!(b[3]["text"].as_str().unwrap().contains("data.bin"));
        // Copied files paste as URIs (spaces encoded) or paths.
        let uri = format!("file://{}", script.to_string_lossy().replace(' ', "%20"));
        assert_eq!(pasted_files(&uri), Some(vec![script.clone()]));
        assert_eq!(pasted_files(&img.to_string_lossy()), Some(vec![img.clone()]));
        assert_eq!(pasted_files("just some words"), None);
        assert_eq!(pasted_files("/no/such/file.pine"), None);
        assert_eq!(icon("a.pine"), ph::FILE_CODE);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

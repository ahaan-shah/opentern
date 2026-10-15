//! The Script Editor tab, laid out like a small IDE: an explorer (the
//! workspace's scripts, plus the read-only Pine source of built-ins), open
//! files as tabs, a code view with line numbers, the current line and error
//! lines marked, Run (Ctrl+Enter: save and put on the chart, or update it),
//! auto-indent, Ctrl+/ comments, a console whose "Line N" errors jump to the
//! line, and a status bar. Files can also be edited in your own editor: the
//! chart reloads when they change.

use crate::app::App;
use crate::icons::ph;
use crate::motion;
use crate::scripting::Lang;
use crate::theme::{self, Theme};
use egui::text::{CCursor, CCursorRange, LayoutJob, TextFormat};
use egui::{Align2, Color32, CornerRadius, FontId, Id, Key, Modifiers, Rect, Sense, Stroke, Ui, pos2, vec2};
use std::time::SystemTime;

/// An open file (a tab).
pub struct Doc {
    /// The file name in the scripts folder, or `builtin:<id>`.
    pub key: String,
    pub title: String,
    pub lang: Lang,
    pub text: String,
    saved: String,
    mtime: Option<SystemTime>,
    pub readonly: bool,
    /// A line to move the cursor to on the next frame (0-based).
    jump: Option<usize>,
}

impl Doc {
    fn dirty(&self) -> bool {
        !self.readonly && self.text != self.saved
    }
}

pub struct Editor {
    pub docs: Vec<Doc>,
    pub active: usize,
    pub status: String,
    filter: String,
    show_builtins: bool,
    /// (file, new name) while a file is being renamed in the explorer.
    renaming: Option<(String, String)>,
    /// A file whose Delete was pressed once (press again to delete).
    deleting: Option<String>,
    explorer_w: f32,
    /// Where the cursor is, for the status bar: (line, column), 1-based.
    caret: (usize, usize),
    /// Tried opening the first script once (an empty editor stays empty
    /// after the last tab is closed).
    started: bool,
}

impl Default for Editor {
    fn default() -> Self {
        Editor {
            docs: Vec::new(),
            active: 0,
            status: String::new(),
            filter: String::new(),
            show_builtins: false,
            renaming: None,
            deleting: None,
            explorer_w: 230.0,
            caret: (1, 1),
            started: false,
        }
    }
}

impl Editor {
    pub fn doc(&self) -> Option<&Doc> {
        self.docs.get(self.active)
    }
}

pub const PYTHON_TEMPLATE: &str = r##"from opentern import *

indicator("My indicator", overlay=True)
length = input.int(20, "Length")
src = input.source("close", "Source")

def compute(bars):
    basis = ta.sma(bars[src], length)
    plot(basis, "Basis", color="#2962FF", linewidth=2)
"##;

pub const PINE_TEMPLATE: &str = r##"//@version=6
indicator("My script", overlay=true)
length = input.int(20, "Length")
src = input.source(close, "Source")
basis = ta.sma(src, length)
plot(basis, "Basis", color=color.blue, linewidth=2)
"##;

pub const LUAU_TEMPLATE: &str = r##"indicator("My script", { overlay = true })
local length = input.int("Length", 20, { min = 1 })
local src = input.source("Source", "close")

-- Runs on every bar. Keep values that carry over to the next bar in `state`.
function on_bar(bar)
  local basis = ta.sma(bar[src], length)
  plot(basis, "Basis", { color = "#2962FF", linewidth = 2 })
end
"##;

pub const JS_TEMPLATE: &str = r##"indicator("My script", { overlay: true });
const length = input.int("Length", 20, { min: 1 });
const src = input.source("Source", "close");

// Runs on every bar. Keep values that carry over to the next bar in `state`.
function onBar(bar) {
  const basis = ta.sma(bar[src], length);
  plot(basis, "Basis", { color: "#2962FF", linewidth: 2 });
}
"##;

pub const TS_TEMPLATE: &str = r##"indicator("My script", { overlay: true });
const length: number = input.int("Length", 20, { min: 1 });
const src: string = input.source("Source", "close");

// Runs on every bar. Keep values that carry over to the next bar in `state`.
function onBar(bar: Record<string, number>): void {
  const basis: number = ta.sma(bar[src], length);
  plot(basis, "Basis", { color: "#2962FF", linewidth: 2 });
}
"##;

const LUAU_KEYWORDS: &[&str] = &[
    "local", "function", "end", "if", "then", "else", "elseif", "for", "in", "do", "while", "repeat", "until",
    "return", "and", "or", "not", "true", "false", "nil", "break", "continue", "type", "export",
];
const JS_KEYWORDS: &[&str] = &[
    "const",
    "let",
    "var",
    "function",
    "return",
    "if",
    "else",
    "for",
    "of",
    "in",
    "while",
    "do",
    "break",
    "continue",
    "true",
    "false",
    "null",
    "undefined",
    "new",
    "typeof",
    "class",
    "switch",
    "case",
    "default",
    "interface",
    "type",
    "as",
    "number",
    "string",
    "boolean",
    "void",
    "export",
    "import",
];

const PY_KEYWORDS: &[&str] = &[
    "def", "return", "if", "elif", "else", "for", "while", "in", "not", "and", "or", "import", "from", "as", "True",
    "False", "None", "lambda", "class", "try", "except", "with", "pass", "break", "continue",
];
const PINE_KEYWORDS: &[&str] = &[
    "if", "else", "for", "to", "by", "while", "switch", "var", "varip", "and", "or", "not", "true", "false", "na",
    "import", "export", "method", "type", "int", "float", "bool", "string", "color", "series", "simple", "const", "in",
    "continue", "break",
];

/// Colours code: comments, strings, numbers, keywords, namespaces.
pub fn highlight(text: &str, lang: Lang, t: &Theme, font: FontId) -> LayoutJob {
    let mut job = LayoutJob::default();
    let fmt = |c: Color32| TextFormat {
        font_id: font.clone(),
        color: c,
        ..Default::default()
    };
    let (comment, keyword, string, number, ns) = if t.dark {
        (
            Color32::from_rgb(0x6A, 0x73, 0x7D),
            Color32::from_rgb(0xC6, 0x78, 0xDD),
            Color32::from_rgb(0x98, 0xC3, 0x79),
            Color32::from_rgb(0xD1, 0x9A, 0x66),
            Color32::from_rgb(0x61, 0xAF, 0xEF),
        )
    } else {
        (
            Color32::from_rgb(0x8A, 0x91, 0x99),
            Color32::from_rgb(0xA6, 0x26, 0xA4),
            Color32::from_rgb(0x50, 0xA1, 0x4F),
            Color32::from_rgb(0x98, 0x68, 0x01),
            Color32::from_rgb(0x40, 0x78, 0xF2),
        )
    };
    let (line_comment, keywords) = match lang {
        Lang::Python => ("#", PY_KEYWORDS),
        Lang::Pine => ("//", PINE_KEYWORDS),
        Lang::Luau => ("--", LUAU_KEYWORDS),
        Lang::Js | Lang::Ts => ("//", JS_KEYWORDS),
    };
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    let mut plain = String::new();
    let flush = |job: &mut LayoutJob, plain: &mut String| {
        if !plain.is_empty() {
            job.append(plain, 0.0, fmt(t.text));
            plain.clear();
        }
    };
    while i < chars.len() {
        let rest: String = chars[i..chars.len().min(i + 2)].iter().collect();
        if rest.starts_with(line_comment) {
            flush(&mut job, &mut plain);
            let end = chars[i..]
                .iter()
                .position(|c| *c == '\n')
                .map(|p| i + p)
                .unwrap_or(chars.len());
            job.append(&chars[i..end].iter().collect::<String>(), 0.0, fmt(comment));
            i = end;
        } else if chars[i] == '"' || chars[i] == '\'' {
            flush(&mut job, &mut plain);
            let q = chars[i];
            let mut j = i + 1;
            while j < chars.len() && chars[j] != q && chars[j] != '\n' {
                if chars[j] == '\\' {
                    j += 1;
                }
                j += 1;
            }
            let end = (j + 1).min(chars.len());
            job.append(&chars[i..end].iter().collect::<String>(), 0.0, fmt(string));
            i = end;
        } else if chars[i].is_ascii_digit() && (i == 0 || !chars[i - 1].is_alphanumeric()) {
            flush(&mut job, &mut plain);
            let mut j = i;
            while j < chars.len() && (chars[j].is_ascii_alphanumeric() || chars[j] == '.') {
                j += 1;
            }
            job.append(&chars[i..j].iter().collect::<String>(), 0.0, fmt(number));
            i = j;
        } else if chars[i].is_alphabetic() || chars[i] == '_' {
            let mut j = i;
            while j < chars.len() && (chars[j].is_alphanumeric() || chars[j] == '_') {
                j += 1;
            }
            let word: String = chars[i..j].iter().collect();
            let dotted = j < chars.len() && chars[j] == '.';
            if keywords.contains(&word.as_str()) {
                flush(&mut job, &mut plain);
                job.append(&word, 0.0, fmt(keyword));
            } else if dotted || chars.get(j) == Some(&'(') {
                flush(&mut job, &mut plain);
                job.append(&word, 0.0, fmt(ns));
            } else {
                plain.push_str(&word);
            }
            i = j;
        } else {
            plain.push(chars[i]);
            i += 1;
        }
    }
    flush(&mut job, &mut plain);
    job
}

/// A short tag and colour per language, for the explorer and the tabs.
fn badge(lang: Lang) -> (&'static str, Color32) {
    match lang {
        Lang::Pine => ("PINE", Color32::from_rgb(0x4C, 0x8D, 0xFF)),
        Lang::Python => ("PY", Color32::from_rgb(0xE0, 0xB0, 0x3A)),
        Lang::Luau => ("LUAU", Color32::from_rgb(0x2E, 0xBF, 0xB0)),
        Lang::Js => ("JS", Color32::from_rgb(0xE8, 0x9F, 0x3A)),
        Lang::Ts => ("TS", Color32::from_rgb(0x3C, 0x8C, 0xE7)),
    }
}

fn lang_name(lang: Lang) -> &'static str {
    match lang {
        Lang::Pine => "Pine Script",
        Lang::Python => "Python",
        Lang::Luau => "Luau",
        Lang::Js => "JavaScript",
        Lang::Ts => "TypeScript",
    }
}

fn indent_unit(lang: Lang) -> &'static str {
    match lang {
        Lang::Pine | Lang::Python => "    ",
        _ => "  ",
    }
}

fn comment_prefix(lang: Lang) -> &'static str {
    match lang {
        Lang::Python => "#",
        Lang::Luau => "--",
        _ => "//",
    }
}

/// The line an error points at (1-based): in a Python traceback the last
/// frame in this file, else the first "Line N".
pub fn error_line(err: &str, file: &str) -> Option<usize> {
    let num = |s: &str| -> Option<usize> {
        let lower = s.to_lowercase();
        let i = lower.find("line ")?;
        lower[i + 5..]
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .collect::<String>()
            .parse()
            .ok()
    };
    let in_file = err.lines().filter(|l| l.contains(file)).filter_map(num).next_back();
    in_file.or_else(|| err.lines().find_map(num))
}

fn byte_at(s: &str, ci: usize) -> usize {
    s.char_indices().nth(ci).map(|(b, _)| b).unwrap_or(s.len())
}

/// Char index of the start of the line holding char `ci`.
fn line_start(chars: &[char], ci: usize) -> usize {
    chars[..ci.min(chars.len())]
        .iter()
        .rposition(|c| *c == '\n')
        .map(|p| p + 1)
        .unwrap_or(0)
}

fn line_end(chars: &[char], ci: usize) -> usize {
    chars[ci.min(chars.len())..]
        .iter()
        .position(|c| *c == '\n')
        .map(|p| ci + p)
        .unwrap_or(chars.len())
}

/// Rewrites the whole lines from char `a` to char `b` with `f`; returns the
/// new block's char range (to select it).
fn edit_lines(text: &mut String, a: usize, b: usize, f: impl Fn(&[&str]) -> Vec<String>) -> (usize, usize) {
    let chars: Vec<char> = text.chars().collect();
    let start = line_start(&chars, a);
    // A selection ending at the start of a line leaves that line alone.
    let b = if b > a && b > 0 && chars.get(b - 1) == Some(&'\n') {
        b - 1
    } else {
        b
    };
    let end = line_end(&chars, b);
    let block: String = chars[start..end].iter().collect();
    let lines: Vec<&str> = block.split('\n').collect();
    let new = f(&lines).join("\n");
    let (bs, be) = (byte_at(text, start), byte_at(text, end));
    text.replace_range(bs..be, &new);
    (start, start + new.chars().count())
}

/// The indentation for a new line typed after `line` (up to the cursor).
fn next_indent(line: &str, lang: Lang) -> String {
    let lead: String = line.chars().take_while(|c| *c == ' ' || *c == '\t').collect();
    let s = line.trim_end();
    let first = s.split_whitespace().next().unwrap_or("");
    let opens = match lang {
        Lang::Python => s.ends_with(':'),
        Lang::Pine => {
            s.ends_with("=>")
                || matches!(first, "if" | "else" | "for" | "while" | "switch" | "type" | "method")
                || (s.ends_with('=') && !s.ends_with("==") && first != "//")
        }
        Lang::Luau => {
            s.ends_with("then")
                || s.ends_with("do")
                || s.ends_with("else")
                || s.ends_with("repeat")
                || (s.contains("function") && s.ends_with(')'))
                || s.ends_with('{')
        }
        Lang::Js | Lang::Ts => s.ends_with('{') || s.ends_with('(') || s.ends_with('['),
    };
    if opens { lead + indent_unit(lang) } else { lead }
}

impl App {
    fn doc_index(&self, key: &str) -> Option<usize> {
        self.editor.docs.iter().position(|d| d.key == key)
    }

    /// Opens a script file in a tab (or switches to its tab).
    pub(crate) fn open_in_editor(&mut self, file: &str) {
        if let Some(i) = self.doc_index(file) {
            self.editor.active = i;
            return;
        }
        let Some(lang) = Lang::of(file) else { return };
        let path = self.scripts_dir().join(file);
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        self.editor.docs.push(Doc {
            key: file.to_string(),
            title: file.to_string(),
            lang,
            saved: text.clone(),
            text,
            mtime: std::fs::metadata(&path).and_then(|m| m.modified()).ok(),
            readonly: false,
            jump: None,
        });
        self.editor.active = self.editor.docs.len() - 1;
        self.editor.status.clear();
    }

    /// Opens a built-in's Pine source, read-only.
    fn open_builtin_source(&mut self, id: &str) {
        let key = format!("builtin:{id}");
        if let Some(i) = self.doc_index(&key) {
            self.editor.active = i;
            return;
        }
        let Some(src) = crate::sources::of(id) else { return };
        let name = opentern_series::builtins::by_id(id).map(|m| m.name).unwrap_or(id);
        self.editor.docs.push(Doc {
            key,
            title: name.to_string(),
            lang: Lang::Pine,
            text: src.to_string(),
            saved: src.to_string(),
            mtime: None,
            readonly: true,
            jump: None,
        });
        self.editor.active = self.editor.docs.len() - 1;
    }

    /// The legend's code button: a script opens its file, a built-in its
    /// Pine source. Opens the Script Editor tab.
    pub(crate) fn open_study_source(&mut self, uid: u64) {
        if let Some(run) = self.scripts.iter().find(|r| r.uid == Some(uid)) {
            let file = run.file.clone();
            self.open_in_editor(&file);
        } else if let Some(s) = self.chart.studies.iter().find(|s| s.uid == uid) {
            let id = s.meta().id;
            if let Some(file) = id.split_once(':').map(|(_, f)| f.to_string()) {
                self.open_in_editor(&file);
            } else {
                self.open_builtin_source(id);
            }
        }
        self.editor.started = true;
        self.layout.tab = Some(crate::shell::Tab::Editor);
    }

    /// Whether a study has source the editor can show.
    pub(crate) fn has_source(id: &str) -> bool {
        id.contains(':') || crate::sources::of(id).is_some()
    }

    fn close_doc(&mut self, i: usize) {
        if i >= self.editor.docs.len() {
            return;
        }
        self.editor.docs.remove(i);
        if self.editor.active > i || self.editor.active >= self.editor.docs.len() {
            self.editor.active = self.editor.active.saturating_sub(1);
        }
    }

    fn new_script(&mut self, lang: Lang) {
        let dir = self.scripts_dir();
        let _ = std::fs::create_dir_all(&dir);
        let ext = lang.ext();
        let name = (1..)
            .map(|k| format!("Untitled {k}{ext}"))
            .find(|n| !dir.join(n).exists())
            .expect("a free name");
        let body = match lang {
            Lang::Python => PYTHON_TEMPLATE,
            Lang::Pine => PINE_TEMPLATE,
            Lang::Luau => LUAU_TEMPLATE,
            Lang::Js => JS_TEMPLATE,
            Lang::Ts => TS_TEMPLATE,
        };
        if std::fs::write(dir.join(&name), body).is_ok() {
            self.open_in_editor(&name);
            self.editor.renaming = Some((name.clone(), name));
        }
    }

    /// A free "<stem> copy.ext"-style name in the scripts folder.
    fn free_name(&self, stem: &str, ext: &str) -> String {
        let dir = self.scripts_dir();
        std::iter::once(format!("{stem}{ext}"))
            .chain((2..).map(|k| format!("{stem} {k}{ext}")))
            .find(|n| !dir.join(n).exists())
            .expect("a free name")
    }

    /// Copies a read-only built-in source into the scripts folder.
    fn copy_builtin(&mut self) {
        let Some(d) = self.editor.doc() else { return };
        let (stem, text) = (d.title.clone(), d.text.clone());
        let name = self.free_name(&stem, ".pine");
        let _ = std::fs::create_dir_all(self.scripts_dir());
        match std::fs::write(self.scripts_dir().join(&name), text) {
            Ok(()) => {
                self.open_in_editor(&name);
                self.editor.status = format!("Copied to {name}.");
            }
            Err(e) => self.editor.status = format!("Couldn't copy it: {e}"),
        }
    }

    fn duplicate_script(&mut self, file: &str) {
        let Some(lang) = Lang::of(file) else { return };
        let stem = file.strip_suffix(lang.ext()).unwrap_or(file);
        let name = self.free_name(&format!("{stem} copy"), lang.ext());
        let dir = self.scripts_dir();
        if std::fs::copy(dir.join(file), dir.join(&name)).is_ok() {
            self.open_in_editor(&name);
        }
    }

    fn rename_script(&mut self, from: &str, to: &str) {
        let to = to.trim();
        let Some(lang) = Lang::of(from) else { return };
        // Keep the language: "My RSI" becomes "My RSI.pine".
        let to = if Lang::of(to).is_some() {
            to.to_string()
        } else {
            format!("{to}{}", lang.ext())
        };
        if to == from || to.contains(['/', '\\']) || to.starts_with('.') {
            return;
        }
        let dir = self.scripts_dir();
        if dir.join(&to).exists() {
            self.editor.status = format!("There's already a script called {to}.");
            return;
        }
        if let Err(e) = std::fs::rename(dir.join(from), dir.join(&to)) {
            self.editor.status = format!("Couldn't rename it: {e}");
            return;
        }
        if let Some(i) = self.doc_index(from) {
            let d = &mut self.editor.docs[i];
            d.key = to.clone();
            d.title = to.clone();
            d.lang = Lang::of(&to).unwrap_or(d.lang);
        }
    }

    fn delete_script(&mut self, file: &str) {
        if std::fs::remove_file(self.scripts_dir().join(file)).is_ok()
            && let Some(i) = self.doc_index(file)
        {
            self.close_doc(i);
        }
    }

    pub(crate) fn save_script(&mut self) {
        let Some(d) = self.editor.docs.get(self.editor.active) else {
            return;
        };
        if d.readonly {
            return;
        }
        let file = d.key.clone();
        let path = self.scripts_dir().join(&file);
        match std::fs::write(&path, &d.text) {
            Ok(()) => {
                let d = &mut self.editor.docs[self.editor.active];
                d.saved = d.text.clone();
                d.mtime = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
                self.editor.status = format!("Saved {file}.");
                // In-process scripts recompile on save; Python reloads by itself.
                for i in 0..self.scripts.len() {
                    if self.scripts[i].file == file && self.scripts[i].lang.in_process() {
                        self.compile_pine(i);
                    }
                }
            }
            Err(e) => self.editor.status = format!("Couldn't save: {e}"),
        }
    }

    /// Run: save, then put the script on the chart or update it there.
    fn run_script(&mut self) {
        let Some(d) = self.editor.doc() else { return };
        if d.readonly {
            return;
        }
        let file = d.key.clone();
        self.save_script();
        if !self.scripts.iter().any(|r| r.file == file) {
            self.add_script(&file);
            self.editor.status = format!("Added {file} to the chart.");
        } else {
            self.editor.status = format!("Updated {file} on the chart.");
        }
    }

    /// Picks up changes made in another editor (when nothing is unsaved here).
    fn sync_editor_files(&mut self) {
        let dir = self.scripts_dir();
        let mut changed = Vec::new();
        for d in self.editor.docs.iter_mut().filter(|d| !d.readonly) {
            let now = std::fs::metadata(dir.join(&d.key)).and_then(|m| m.modified()).ok();
            if now.is_some() && now != d.mtime && d.text == d.saved {
                d.text = std::fs::read_to_string(dir.join(&d.key)).unwrap_or_default();
                d.saved = d.text.clone();
                d.mtime = now;
                changed.push(d.key.clone());
            }
        }
        for file in changed {
            self.editor.status = format!("Reloaded {file}: it changed on disk.");
            for i in 0..self.scripts.len() {
                if self.scripts[i].file == file && self.scripts[i].lang.in_process() {
                    self.compile_pine(i);
                }
            }
        }
    }

    pub(crate) fn editor_tab(&mut self, ui: &mut Ui, t: &Theme) {
        self.sync_editor_files();
        let files = self.script_files();
        if !self.editor.started {
            self.editor.started = true;
            if self.editor.docs.is_empty()
                && let Some(f) = files.first()
            {
                self.open_in_editor(&f.clone());
            }
        }
        // Shortcuts that work anywhere in the tab.
        let (run, save, close) = ui.input_mut(|i| {
            (
                i.consume_key(Modifiers::COMMAND, Key::Enter),
                i.consume_key(Modifiers::COMMAND, Key::S),
                i.consume_key(Modifiers::COMMAND, Key::W),
            )
        });
        if run {
            self.run_script();
        } else if save {
            self.save_script();
        }
        if close {
            let a = self.editor.active;
            if self.editor.doc().is_some_and(|d| !d.dirty()) {
                self.close_doc(a);
            }
        }

        let full = ui.max_rect();
        let ew = self.editor.explorer_w.clamp(170.0, (full.width() * 0.4).max(170.0));
        let explorer = Rect::from_min_size(full.min, vec2(ew, full.height()));
        let main = Rect::from_min_max(pos2(explorer.right() + 12.0, full.top()), full.max);
        let d = crate::widgets::drag_handle(
            ui,
            t,
            Rect::from_min_max(
                pos2(explorer.right() + 2.0, full.top()),
                pos2(main.left() - 2.0, full.bottom()),
            ),
            "explorer-resize",
            true,
        );
        if d != 0.0 {
            self.editor.explorer_w = (ew + d).clamp(170.0, 420.0);
        }
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(explorer));
        child.set_clip_rect(explorer.intersect(ui.clip_rect()));
        self.explorer(&mut child, t, &files);
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(main));
        child.set_clip_rect(main.intersect(ui.clip_rect()));
        self.editor_main(&mut child, t);
    }

    fn explorer(&mut self, ui: &mut Ui, t: &Theme, files: &[String]) {
        use crate::widgets as w;
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new("EXPLORER")
                    .font(theme::semibold(11.0))
                    .color(t.text3),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                if w::tool_button(ui, t, ph::FOLDER_OPEN, "Open the scripts folder", false, 26.0).clicked() {
                    let dir = self.scripts_dir();
                    let _ = std::fs::create_dir_all(&dir);
                    crate::app::open_path(&dir);
                }
                let resp = w::tool_button(ui, t, ph::PLUS, "New script", false, 26.0);
                egui::Popup::menu(&resp).show(|ui| {
                    ui.set_width(200.0);
                    ui.spacing_mut().item_spacing.y = 1.0;
                    for lang in [Lang::Pine, Lang::Python, Lang::Luau, Lang::Js, Lang::Ts] {
                        if w::menu_item(ui, t, Some(ph::FILE_CODE), lang_name(lang), lang.ext(), false).clicked() {
                            self.new_script(lang);
                            ui.close();
                        }
                    }
                });
            });
        });
        ui.add_space(2.0);
        w::search_field(ui, t, &mut self.editor.filter, "Filter", ui.available_width());
        ui.add_space(6.0);
        let q = self.editor.filter.trim().to_lowercase();
        let active_key = self.editor.doc().map(|d| d.key.clone());
        let mut open = None;
        let mut action: Option<(u8, String)> = None;
        egui::ScrollArea::vertical()
            .id_salt("explorer")
            .auto_shrink([false; 2])
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 1.0;
                let shown: Vec<&String> = files
                    .iter()
                    .filter(|f| q.is_empty() || f.to_lowercase().contains(&q))
                    .collect();
                section(ui, t, "SCRIPTS", Some(shown.len()));
                if files.is_empty() {
                    ui.add_space(4.0);
                    ui.label(
                        egui::RichText::new("No scripts yet. Press + to start one.")
                            .font(theme::regular(12.0))
                            .color(t.text3),
                    );
                }
                for f in shown {
                    let lang = Lang::of(f).unwrap_or(Lang::Pine);
                    let on_chart = self.scripts.iter().any(|r| &r.file == f);
                    let selected = active_key.as_deref() == Some(f.as_str());
                    if let Some((from, to)) = &mut self.editor.renaming
                        && from == f
                    {
                        let r = ui.add(
                            w::field(to)
                                .desired_width(ui.available_width())
                                .font(theme::regular(12.5))
                                .margin(vec2(8.0, 4.0)),
                        );
                        if !r.has_focus() && !r.lost_focus() {
                            r.request_focus();
                        }
                        let (enter, esc) = ui.input(|i| (i.key_pressed(Key::Enter), i.key_pressed(Key::Escape)));
                        if r.lost_focus() || enter || esc {
                            if !esc {
                                action = Some((3, to.clone()));
                            }
                            if action.is_none() {
                                action = Some((9, String::new()));
                            }
                        }
                        continue;
                    }
                    let resp = file_row(ui, t, lang, f, selected, on_chart, false);
                    if resp.clicked() {
                        open = Some(f.clone());
                    }
                    if resp.double_clicked() {
                        self.editor.renaming = Some((f.clone(), f.clone()));
                    }
                    resp.context_menu(|ui| {
                        ui.set_width(200.0);
                        ui.spacing_mut().item_spacing.y = 1.0;
                        let mut item = |ui: &mut Ui, icon, label: &str, what: u8| {
                            if w::menu_item(ui, t, Some(icon), label, "", false).clicked() {
                                action = Some((what, f.clone()));
                                ui.close();
                            }
                        };
                        item(
                            ui,
                            ph::CHART_LINE_UP,
                            if on_chart { "Update on chart" } else { "Add to chart" },
                            1,
                        );
                        item(ui, ph::COPY, "Duplicate", 2);
                        if !on_chart {
                            item(ui, ph::PENCIL_SIMPLE, "Rename", 4);
                        }
                        item(ui, ph::ARROW_SQUARE_OUT, "Open in your editor", 5);
                        if !on_chart {
                            w::menu_rule(ui, t);
                            if w::menu_danger(ui, t, Some(ph::TRASH), "Delete").clicked() {
                                action = Some((6, f.clone()));
                                ui.close();
                            }
                        }
                    });
                }
                // Built-ins' Pine source, read-only.
                ui.add_space(10.0);
                let list: Vec<(&str, &str)> = crate::sources::SOURCES
                    .iter()
                    .filter_map(|(id, _)| opentern_series::builtins::by_id(id).map(|m| (*id, m.name)))
                    .filter(|(_, n)| q.is_empty() || n.to_lowercase().contains(&q))
                    .collect();
                let open_list = self.editor.show_builtins || !q.is_empty();
                let caret = if open_list { ph::CARET_DOWN } else { ph::CARET_RIGHT };
                let head = section(ui, t, &format!("BUILT-INS  {caret}"), Some(list.len()));
                if head.clicked() {
                    self.editor.show_builtins = !self.editor.show_builtins;
                }
                if open_list {
                    for (id, name) in list {
                        let key = format!("builtin:{id}");
                        let selected = active_key.as_deref() == Some(key.as_str());
                        if file_row(ui, t, Lang::Pine, name, selected, false, true).clicked() {
                            open = Some(key);
                        }
                    }
                }
            });
        if let Some(k) = open {
            match k.strip_prefix("builtin:") {
                Some(id) => self.open_builtin_source(id),
                None => self.open_in_editor(&k),
            }
        }
        match action {
            Some((1, f)) => {
                self.open_in_editor(&f);
                self.run_script();
            }
            Some((2, f)) => self.duplicate_script(&f),
            Some((3, to)) => {
                if let Some((from, _)) = self.editor.renaming.take() {
                    self.rename_script(&from, &to);
                }
            }
            Some((4, f)) => self.editor.renaming = Some((f.clone(), f)),
            Some((5, f)) => crate::app::open_path(&self.scripts_dir().join(&f)),
            Some((6, f)) => {
                if self.editor.deleting.as_deref() == Some(f.as_str()) {
                    self.editor.deleting = None;
                    self.delete_script(&f);
                    self.editor.status = format!("Deleted {f}.");
                } else {
                    self.editor.status = format!("Delete {f}? Choose Delete again to confirm.");
                    self.editor.deleting = Some(f);
                }
            }
            Some((9, _)) => self.editor.renaming = None,
            _ => {}
        }
    }

    fn editor_main(&mut self, ui: &mut Ui, t: &Theme) {
        let full = ui.max_rect();
        // Tabs and the toolbar.
        let bar = Rect::from_min_size(full.min, vec2(full.width(), 32.0));
        let tools_w = 300.0_f32.min(full.width() * 0.5);
        let tabs_rect = Rect::from_min_max(bar.min, pos2(bar.right() - tools_w, bar.bottom()));
        self.doc_tabs(ui, t, tabs_rect);
        let mut tools = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(Rect::from_min_max(pos2(tabs_rect.right(), bar.top()), bar.max))
                .layout(egui::Layout::right_to_left(egui::Align::Center)),
        );
        self.editor_tools(&mut tools, t);
        ui.painter().hline(
            full.x_range(),
            bar.bottom() + 0.5,
            Stroke::new(1.0, t.border.gamma_multiply(0.7)),
        );

        // Console and status bar at the bottom.
        let console = self.console_text(t);
        let console_lines = console.as_ref().map(|c| c.0.lines().count().min(5)).unwrap_or(1);
        let console_h = 26.0 + 16.0 * console_lines.saturating_sub(1) as f32;
        let status_h = 22.0;
        let code = Rect::from_min_max(
            pos2(full.left(), bar.bottom() + 8.0),
            pos2(full.right(), full.bottom() - console_h - status_h - 8.0),
        );
        if self.editor.docs.is_empty() {
            let mut c = ui.new_child(egui::UiBuilder::new().max_rect(code));
            crate::widgets::empty_state(
                &mut c,
                t,
                ph::CODE,
                "No file open",
                "Pick a script in the explorer, or start one with +.",
            );
        } else if code.height() > 30.0 {
            self.code_view(ui, t, code);
        }
        let crect = Rect::from_min_max(
            pos2(full.left(), code.bottom() + 6.0),
            pos2(full.right(), code.bottom() + 6.0 + console_h),
        );
        let mut c = ui.new_child(egui::UiBuilder::new().max_rect(crect));
        c.set_clip_rect(crect.intersect(ui.clip_rect()));
        self.console(&mut c, t, console);
        let srect = Rect::from_min_max(pos2(full.left(), full.bottom() - status_h), full.max);
        self.status_bar(ui, t, srect);
    }

    fn doc_tabs(&mut self, ui: &mut Ui, t: &Theme, rect: Rect) {
        let mut x = rect.left();
        let mut pick = None;
        let mut close = None;
        let font = theme::medium(12.5);
        for (i, d) in self.editor.docs.iter().enumerate() {
            let (tag, col) = badge(d.lang);
            let name = ui.painter().layout_no_wrap(d.title.clone(), font.clone(), t.text);
            let tag_g = ui
                .painter()
                .layout_no_wrap(tag.to_string(), FontId::monospace(9.5), col);
            let w = (tag_g.size().x + name.size().x + 52.0).min(220.0);
            if x + w > rect.right() {
                break;
            }
            let r = Rect::from_min_size(pos2(x, rect.top()), vec2(w, rect.height()));
            x += w + 2.0;
            let resp = ui.interact(r, Id::new(("doc-tab", &d.key)), Sense::click());
            let on = i == self.editor.active;
            let inside = ui.rect_contains_pointer(r);
            let p = ui.painter().with_clip_rect(r);
            if on {
                p.rect_filled(r.shrink2(vec2(0.0, 2.0)), CornerRadius::same(6), t.hover_wash());
                p.hline(
                    r.shrink2(vec2(8.0, 0.0)).x_range(),
                    r.bottom() - 1.0,
                    Stroke::new(2.0, t.accent),
                );
            } else if inside {
                p.rect_filled(r.shrink2(vec2(0.0, 2.0)), CornerRadius::same(6), t.hover_wash());
            }
            let cy = r.center().y;
            p.galley(pos2(r.left() + 10.0, cy - tag_g.size().y / 2.0), tag_g.clone(), col);
            let nx = r.left() + 16.0 + tag_g.size().x;
            p.galley_with_override_text_color(
                pos2(nx, cy - name.size().y / 2.0),
                name,
                if on { t.text } else { t.text2 },
            );
            if d.readonly {
                p.text(
                    pos2(r.right() - 32.0, cy),
                    Align2::CENTER_CENTER,
                    ph::LOCK_SIMPLE,
                    theme::regular(11.0),
                    t.text3,
                );
            }
            let xr = Rect::from_center_size(pos2(r.right() - 14.0, cy), vec2(18.0, 18.0));
            let xresp = ui.interact(xr, Id::new(("doc-x", &d.key)), Sense::click());
            if d.dirty() && !inside {
                p.circle_filled(xr.center(), 3.5, t.text2);
            } else if inside || on {
                if xresp.hovered() {
                    p.rect_filled(xr, CornerRadius::same(4), t.hover_wash());
                }
                p.text(xr.center(), Align2::CENTER_CENTER, ph::X, theme::regular(11.0), t.text2);
            }
            if xresp.on_hover_text("Close (Ctrl+W)").clicked() || resp.middle_clicked() {
                close = Some(i);
            } else if resp.clicked() {
                pick = Some(i);
            }
        }
        if let Some(i) = pick {
            self.editor.active = i;
        }
        if let Some(i) = close {
            let d = &self.editor.docs[i];
            if d.dirty() && self.editor.deleting.as_deref() != Some(d.key.as_str()) {
                self.editor.status = format!(
                    "{} has unsaved changes: close again to discard them, or save first.",
                    d.title
                );
                self.editor.deleting = Some(d.key.clone());
            } else {
                self.editor.deleting = None;
                self.close_doc(i);
            }
        }
    }

    fn editor_tools(&mut self, ui: &mut Ui, t: &Theme) {
        use crate::widgets as w;
        let Some(d) = self.editor.doc() else { return };
        ui.spacing_mut().item_spacing.x = 4.0;
        if d.readonly {
            if w::button(ui, w::Kind::Primary, true, Some(ph::COPY), "Make a copy")
                .on_hover_text("Copy this source into your scripts, to change it")
                .clicked()
            {
                self.copy_builtin();
            }
            return;
        }
        let (file, dirty) = (d.key.clone(), d.dirty());
        let on_chart = self.scripts.iter().any(|r| r.file == file);
        let label = if on_chart { "Update chart" } else { "Add to chart" };
        if w::button(ui, w::Kind::Primary, true, Some(ph::PLAY), label)
            .on_hover_text(concat!("Save and run on the chart (", shortcut!("Enter"), ")"))
            .clicked()
        {
            self.run_script();
        }
        if w::tool_button(
            ui,
            t,
            ph::ARROW_SQUARE_OUT,
            "Edit in your own editor; the chart reloads when you save there",
            false,
            30.0,
        )
        .clicked()
        {
            crate::app::open_path(&self.scripts_dir().join(&file));
        }
        let save = w::tool_button(
            ui,
            t,
            ph::FLOPPY_DISK,
            concat!("Save (", shortcut!("S"), ")"),
            false,
            30.0,
        );
        if dirty {
            ui.painter()
                .circle_filled(save.rect.right_top() + vec2(-6.0, 6.0), 3.0, t.accent);
        }
        if save.clicked() {
            self.save_script();
        }
    }

    fn code_view(&mut self, ui: &mut Ui, t: &Theme, rect: Rect) {
        let a = self.editor.active;
        let Some(doc) = self.editor.docs.get(a) else { return };
        let key = doc.key.clone();
        let lang = doc.lang;
        let readonly = doc.readonly;
        let err_line = self
            .scripts
            .iter()
            .find(|r| r.file == key)
            .and_then(|r| r.error.as_deref())
            .and_then(|e| error_line(e, &key));
        let p = ui.painter();
        p.rect(
            rect,
            CornerRadius::same(8),
            t.bg,
            Stroke::new(1.0, t.border.gamma_multiply(0.8)),
            egui::StrokeKind::Inside,
        );
        let id = Id::new(("code", &key));
        let font = FontId::monospace(13.0);
        let row_h = ui.fonts_mut(|f| f.row_height(&font));
        let lines = doc.text.split('\n').count();
        let digits = lines.to_string().len().max(2) as f32;
        let gutter_w = digits * 8.0 + 26.0;
        // Gutter background.
        let gutter = Rect::from_min_size(rect.min, vec2(gutter_w, rect.height()));
        p.rect_filled(
            gutter.shrink(1.0),
            CornerRadius {
                nw: 7,
                sw: 7,
                ne: 0,
                se: 0,
            },
            t.hover_wash().gamma_multiply(0.6),
        );

        // Editing keys, before the text field sees them.
        let focused = ui.memory(|m| m.has_focus(id));
        if focused && !readonly {
            let (untab, tab, enter, comment) = ui.input_mut(|i| {
                (
                    i.consume_key(Modifiers::SHIFT, Key::Tab),
                    i.consume_key(Modifiers::NONE, Key::Tab),
                    i.consume_key(Modifiers::NONE, Key::Enter),
                    i.consume_key(Modifiers::COMMAND, Key::Slash),
                )
            });
            if untab || tab || enter || comment {
                self.edit_keys(ui, id, untab, tab, enter, comment);
            }
        }
        let doc = &mut self.editor.docs[a];
        if let Some(line) = doc.jump.take() {
            let ci: usize = doc.text.split('\n').take(line).map(|l| l.chars().count() + 1).sum();
            let mut state = egui::TextEdit::load_state(ui.ctx(), id).unwrap_or_default();
            state.cursor.set_char_range(Some(CCursorRange::one(CCursor::new(ci))));
            state.store(ui.ctx(), id);
            ui.memory_mut(|m| m.request_focus(id));
            ui.data_mut(|d| d.insert_temp(id.with("scroll-to"), line));
        }

        let inner = rect.shrink(1.0);
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(inner));
        child.set_clip_rect(inner.intersect(ui.clip_rect()));
        let tt = *t;
        let mut layouter = |ui: &Ui, text: &dyn egui::TextBuffer, _wrap: f32| {
            let mut job = highlight(text.as_str(), lang, &tt, FontId::monospace(13.0));
            job.wrap.max_width = f32::INFINITY;
            ui.ctx().fonts_mut(|f| f.layout_job(job))
        };
        let mut caret = None;
        egui::ScrollArea::both()
            .id_salt(("code-scroll", &key))
            .auto_shrink([false; 2])
            .show(&mut child, |ui| {
                let view = ui.clip_rect();
                let bg = ui.painter().add(egui::Shape::Noop);
                let fg = ui.painter().add(egui::Shape::Noop);
                ui.horizontal_top(|ui| {
                    ui.add_space(gutter_w - 1.0);
                    let width = (view.width() - gutter_w - 4.0).max(100.0);
                    let rows = ((view.height() - 12.0) / row_h).max(1.0) as usize;
                    let out = if readonly {
                        let mut ro: &str = doc.text.as_str();
                        egui::TextEdit::multiline(&mut ro)
                            .id(id)
                            .frame(egui::Frame::NONE)
                            .code_editor()
                            .desired_width(width)
                            .desired_rows(rows)
                            .margin(vec2(8.0, 6.0))
                            .layouter(&mut layouter)
                            .show(ui)
                    } else {
                        egui::TextEdit::multiline(&mut doc.text)
                            .id(id)
                            .frame(egui::Frame::NONE)
                            .code_editor()
                            .lock_focus(true)
                            .desired_width(width)
                            .desired_rows(rows)
                            .margin(vec2(8.0, 6.0))
                            .layouter(&mut layouter)
                            .show(ui)
                    };
                    let g = &out.galley;
                    let origin = out.galley_pos;
                    let full_x = view.x_range();
                    let mut shapes = Vec::new();
                    let mut marks = Vec::new();
                    // The current line, and the line an error points at.
                    let cursor_row = out.cursor_range.map(|c| {
                        let r = g.pos_from_cursor(c.primary);
                        let ci = c.primary.index.0;
                        let before: String = doc.text.chars().take(ci).collect();
                        let line = before.matches('\n').count();
                        let col = before.rsplit('\n').next().map(|l| l.chars().count()).unwrap_or(0);
                        caret = Some((line + 1, col + 1));
                        (r.translate(origin.to_vec2()), line)
                    });
                    if let Some((r, _)) = cursor_row
                        && out.response.response.has_focus()
                    {
                        let band = Rect::from_x_y_ranges(full_x, r.y_range());
                        shapes.push(egui::Shape::rect_filled(band, 0.0, tt.hover_wash().gamma_multiply(0.7)));
                    }
                    for (line, row) in g.rows.iter().enumerate() {
                        let y = origin.y + row.pos.y;
                        if y + row_h < view.top() || y > view.bottom() {
                            continue;
                        }
                        let is_err = err_line == Some(line + 1);
                        if is_err {
                            let band = Rect::from_x_y_ranges(full_x, y..=y + row_h);
                            shapes.push(egui::Shape::rect_filled(band, 0.0, tt.neg.gamma_multiply(0.12)));
                        }
                        let cur = cursor_row.is_some_and(|(_, l)| l == line);
                        let col = if is_err {
                            tt.neg
                        } else if cur {
                            tt.text
                        } else {
                            tt.text3
                        };
                        let num = ui
                            .painter()
                            .layout_no_wrap((line + 1).to_string(), FontId::monospace(11.5), col);
                        let at = pos2(
                            view.left() + gutter_w - 12.0 - num.size().x,
                            y + (row_h - num.size().y) / 2.0,
                        );
                        marks.push(egui::Shape::galley(at, num, col));
                    }
                    ui.painter().set(bg, egui::Shape::Vec(shapes));
                    // Numbers stay put while the code scrolls sideways.
                    ui.painter().set(fg, egui::Shape::Vec(marks));
                    if let Some(line) = ui.data_mut(|d| d.remove_temp::<usize>(id.with("scroll-to")))
                        && let Some(row) = g.rows.get(line)
                    {
                        let y = origin.y + row.pos.y;
                        ui.scroll_to_rect(Rect::from_x_y_ranges(full_x, y..=y + row_h), Some(egui::Align::Center));
                    }
                });
            });
        if let Some(c) = caret {
            self.editor.caret = c;
        }
    }

    /// Tab / Shift+Tab indent, Enter keeps (and opens) indentation, Ctrl+/
    /// comments lines.
    fn edit_keys(&mut self, ui: &Ui, id: Id, untab: bool, tab: bool, enter: bool, comment: bool) {
        let a = self.editor.active;
        let doc = &mut self.editor.docs[a];
        let lang = doc.lang;
        let mut state = egui::TextEdit::load_state(ui.ctx(), id).unwrap_or_default();
        let n = doc.text.chars().count();
        let (s, e) = state
            .cursor
            .char_range()
            .map(|r| {
                let (x, y) = (r.primary.index.0.min(n), r.secondary.index.0.min(n));
                (x.min(y), x.max(y))
            })
            .unwrap_or((n, n));
        let unit = indent_unit(lang);
        let multi = doc.text.chars().skip(s).take(e - s).any(|c| c == '\n');
        let new = if enter {
            let chars: Vec<char> = doc.text.chars().collect();
            let ls = line_start(&chars, s);
            let line: String = chars[ls..s].iter().collect();
            let ins = format!("\n{}", next_indent(&line, lang));
            let (bs, be) = (byte_at(&doc.text, s), byte_at(&doc.text, e));
            doc.text.replace_range(bs..be, &ins);
            let c = s + ins.chars().count();
            (c, c)
        } else if tab && !multi {
            let (bs, be) = (byte_at(&doc.text, s), byte_at(&doc.text, e));
            doc.text.replace_range(bs..be, unit);
            let c = s + unit.len();
            (c, c)
        } else if tab || untab {
            edit_lines(&mut doc.text, s, e, |ls| {
                ls.iter()
                    .map(|l| {
                        if tab {
                            if l.is_empty() {
                                l.to_string()
                            } else {
                                format!("{unit}{l}")
                            }
                        } else if let Some(r) = l.strip_prefix(unit) {
                            r.to_string()
                        } else if let Some(r) = l.strip_prefix('\t') {
                            r.to_string()
                        } else {
                            l.trim_start_matches(' ').to_string()
                        }
                    })
                    .collect()
            })
        } else if comment {
            let pre = comment_prefix(lang);
            edit_lines(&mut doc.text, s, e, |ls| {
                let all = ls
                    .iter()
                    .filter(|l| !l.trim().is_empty())
                    .all(|l| l.trim_start().starts_with(pre));
                let indent = ls
                    .iter()
                    .filter(|l| !l.trim().is_empty())
                    .map(|l| l.len() - l.trim_start().len())
                    .min()
                    .unwrap_or(0);
                ls.iter()
                    .map(|l| {
                        if l.trim().is_empty() {
                            l.to_string()
                        } else if all {
                            let k = l.len() - l.trim_start().len();
                            let rest = &l[k + pre.len()..];
                            format!("{}{}", &l[..k], rest.strip_prefix(' ').unwrap_or(rest))
                        } else {
                            format!("{}{pre} {}", &l[..indent], &l[indent..])
                        }
                    })
                    .collect()
            })
        } else {
            return;
        };
        let range = if new.0 == new.1 {
            CCursorRange::one(CCursor::new(new.0))
        } else {
            CCursorRange::two(CCursor::new(new.0), CCursor::new(new.1))
        };
        state.cursor.set_char_range(Some(range));
        state.store(ui.ctx(), id);
    }

    /// The console's text and colour for the open file.
    fn console_text(&self, t: &Theme) -> Option<(String, Color32, bool)> {
        let d = self.editor.doc()?;
        if d.readonly {
            return Some((
                "Built-in source, read-only. Make a copy to change it and run it.".into(),
                t.text3,
                false,
            ));
        }
        let run = self.scripts.iter().find(|r| r.file == d.key);
        Some(match run {
            Some(r) => match &r.error {
                Some(e) => (e.trim().to_string(), t.neg, true),
                None if !r.log.trim().is_empty() => (r.log.trim().to_string(), t.text2, false),
                None => ("Running on the chart. No errors.".into(), t.pos, false),
            },
            None => (
                format!("Not on the chart. {} saves it and puts it there.", shortcut!("Enter")),
                t.text3,
                false,
            ),
        })
    }

    fn console(&mut self, ui: &mut Ui, t: &Theme, console: Option<(String, Color32, bool)>) {
        let r = ui.max_rect();
        ui.painter()
            .rect_filled(r, CornerRadius::same(6), t.hover_wash().gamma_multiply(0.5));
        let Some((text, col, is_err)) = console else { return };
        let icon = if is_err {
            ph::WARNING_CIRCLE
        } else if col == t.pos {
            ph::CHECK_CIRCLE
        } else {
            ph::TERMINAL_WINDOW
        };
        let key = self.editor.doc().map(|d| d.key.clone()).unwrap_or_default();
        let line = is_err.then(|| error_line(&text, &key)).flatten();
        let mut jump = false;
        egui::ScrollArea::both()
            .id_salt("script-console")
            .auto_shrink([false; 2])
            .show(ui, |ui| {
                ui.horizontal_top(|ui| {
                    ui.add_space(8.0);
                    ui.vertical(|ui| {
                        ui.add_space(5.0);
                        ui.label(egui::RichText::new(icon).font(theme::regular(13.0)).color(col));
                    });
                    ui.vertical(|ui| {
                        ui.add_space(5.0);
                        let resp = ui.add(
                            egui::Label::new(egui::RichText::new(&text).font(FontId::monospace(12.0)).color(col))
                                .extend()
                                .sense(if line.is_some() { Sense::click() } else { Sense::hover() }),
                        );
                        if let Some(n) = line {
                            jump = resp
                                .on_hover_text(format!("Go to line {n}"))
                                .on_hover_cursor(egui::CursorIcon::PointingHand)
                                .clicked();
                        }
                    });
                });
            });
        if jump
            && let Some(n) = line
            && let Some(d) = self.editor.docs.get_mut(self.editor.active)
        {
            d.jump = Some(n.saturating_sub(1));
        }
    }

    fn status_bar(&mut self, ui: &mut Ui, t: &Theme, r: Rect) {
        let p = ui.painter();
        let y = r.center().y;
        let font = theme::regular(11.5);
        p.text(
            pos2(r.left() + 4.0, y),
            Align2::LEFT_CENTER,
            &self.editor.status,
            font.clone(),
            t.text3,
        );
        let Some(d) = self.editor.doc() else { return };
        let (line, col) = self.editor.caret;
        let right = format!(
            "Ln {line}, Col {col}    {}    {}",
            if indent_unit(d.lang).len() == 4 {
                "Spaces: 4"
            } else {
                "Spaces: 2"
            },
            lang_name(d.lang)
        );
        p.text(pos2(r.right() - 4.0, y), Align2::RIGHT_CENTER, right, font, t.text3);
    }
}

/// A small caps heading in the explorer; clickable (for folding).
fn section(ui: &mut Ui, t: &Theme, title: &str, count: Option<usize>) -> egui::Response {
    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::click());
    let p = ui.painter();
    p.text(
        pos2(r.left() + 4.0, r.center().y),
        Align2::LEFT_CENTER,
        title,
        theme::semibold(10.5),
        t.text3,
    );
    if let Some(n) = count {
        p.text(
            pos2(r.right() - 6.0, r.center().y),
            Align2::RIGHT_CENTER,
            n.to_string(),
            theme::regular(10.5),
            t.text3,
        );
    }
    resp
}

/// One file in the explorer: language tag, name, and a dot when it's on
/// the chart.
fn file_row(
    ui: &mut Ui,
    t: &Theme,
    lang: Lang,
    name: &str,
    selected: bool,
    on_chart: bool,
    locked: bool,
) -> egui::Response {
    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 26.0), Sense::click());
    let hov = motion::toggle(ui.ctx(), resp.id, ui.rect_contains_pointer(r), motion::MICRO);
    let p = ui.painter();
    p.rect_filled(r, CornerRadius::same(6), motion::with_alpha(t.hover_wash(), hov));
    if selected {
        p.rect_filled(r, CornerRadius::same(6), t.accent_soft());
    }
    let (tag, col) = badge(lang);
    let y = r.center().y;
    let tag_r = Rect::from_min_size(pos2(r.left() + 6.0, y - 8.0), vec2(34.0, 16.0));
    p.rect_filled(tag_r, CornerRadius::same(4), col.gamma_multiply(0.14));
    p.text(tag_r.center(), Align2::CENTER_CENTER, tag, FontId::monospace(9.5), col);
    let right_pad = if on_chart || locked { 22.0 } else { 6.0 };
    let font = theme::regular(12.5);
    let room = r.right() - right_pad - (tag_r.right() + 8.0);
    let shown = crate::format::elide_to(p, name, &font, room);
    p.text(
        pos2(tag_r.right() + 8.0, y),
        Align2::LEFT_CENTER,
        shown,
        font,
        if selected { t.accent } else { t.text },
    );
    if on_chart {
        p.circle_filled(pos2(r.right() - 11.0, y), 3.0, t.pos);
    } else if locked {
        p.text(
            pos2(r.right() - 11.0, y),
            Align2::CENTER_CENTER,
            ph::LOCK_SIMPLE,
            theme::regular(11.0),
            t.text3,
        );
    }
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    if on_chart {
        resp.on_hover_text("On the chart")
    } else {
        resp
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn highlighting_keeps_every_character() {
        let t = crate::theme::Theme::by_name("Opentern Dark");
        for (src, lang) in [
            (PYTHON_TEMPLATE, Lang::Python),
            (PINE_TEMPLATE, Lang::Pine),
            (LUAU_TEMPLATE, Lang::Luau),
            (JS_TEMPLATE, Lang::Js),
            (TS_TEMPLATE, Lang::Ts),
        ] {
            let job = highlight(src, lang, &t, FontId::monospace(13.0));
            assert_eq!(job.text, src);
            assert!(job.sections.len() > 10);
        }
    }
}

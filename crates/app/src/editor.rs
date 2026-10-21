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
    /// The explorer is showing (Ctrl+B).
    sidebar: bool,
    /// The explorer's filter field is open.
    filtering: bool,
    scripts_open: bool,
    outline_open: bool,
    /// Problems / Output under the code: open, and which (0, 1).
    panel_open: bool,
    panel_tab: u8,
    /// Scroll the code here next frame (a minimap click).
    minimap_to: Option<f32>,
    /// The last error shown, so a new one opens Problems once.
    seen_error: Option<String>,
    /// Folders open in the explorer (paths in the scripts folder).
    open_dirs: std::collections::HashSet<String>,
    /// Where new scripts go: the folder last picked ("" is the top).
    new_dir: String,
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
            explorer_w: 250.0,
            caret: (1, 1),
            sidebar: true,
            filtering: false,
            scripts_open: true,
            outline_open: true,
            panel_open: false,
            panel_tab: 0,
            minimap_to: None,
            seen_error: None,
            open_dirs: Default::default(),
            new_dir: String::new(),
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

/// The editor's colours, after Catppuccin: Mocha on dark themes, Latte on
/// light ones. The backgrounds come from the app's theme so the editor sits
/// in it.
#[derive(Clone, Copy)]
pub struct CodeColors {
    pub bg: Color32,
    pub side: Color32,
    pub line: Color32,
    pub current: Color32,
    pub guide: Color32,
    pub text: Color32,
    pub text2: Color32,
    pub dim: Color32,
    pub keyword: Color32,
    pub func: Color32,
    pub ns: Color32,
    pub string: Color32,
    pub number: Color32,
    pub comment: Color32,
    pub op: Color32,
    pub punct: Color32,
}

pub fn code_colors(t: &Theme) -> CodeColors {
    let rgb = |v: u32| Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8);
    let base = CodeColors {
        bg: t.bg,
        side: t.sidebar,
        line: t.border.gamma_multiply(0.7),
        current: t.text.gamma_multiply(0.045),
        guide: t.text.gamma_multiply(0.08),
        text: t.text,
        text2: t.text2,
        dim: t.text3,
        keyword: rgb(0),
        func: rgb(0),
        ns: rgb(0),
        string: rgb(0),
        number: rgb(0),
        comment: rgb(0),
        op: rgb(0),
        punct: rgb(0),
    };
    if t.dark {
        CodeColors {
            text: rgb(0xCDD6F4),
            keyword: rgb(0xCBA6F7),
            func: rgb(0x89B4FA),
            ns: rgb(0x94E2D5),
            string: rgb(0xA6E3A1),
            number: rgb(0xFAB387),
            comment: rgb(0x6C7086),
            op: rgb(0x89DCEB),
            punct: rgb(0x9399B2),
            ..base
        }
    } else {
        CodeColors {
            text: rgb(0x4C4F69),
            keyword: rgb(0x8839EF),
            func: rgb(0x1E66F5),
            ns: rgb(0x179299),
            string: rgb(0x40A02B),
            number: rgb(0xFE640B),
            comment: rgb(0x9CA0B0),
            op: rgb(0x04A5E5),
            punct: rgb(0x7C7F93),
            ..base
        }
    }
}

/// Constants each language spells out.
fn constants(lang: Lang) -> &'static [&'static str] {
    match lang {
        Lang::Python => &["True", "False", "None"],
        Lang::Pine => &["true", "false", "na"],
        Lang::Luau => &["true", "false", "nil"],
        Lang::Js | Lang::Ts => &["true", "false", "null", "undefined", "NaN", "Infinity"],
    }
}

/// Colours code: comments (italic), strings, numbers, keywords, constants,
/// namespaces, function calls, operators and punctuation.
pub fn highlight(text: &str, lang: Lang, t: &Theme, font: FontId) -> LayoutJob {
    let c = code_colors(t);
    let mut job = LayoutJob::default();
    let fmt = |col: Color32, italics: bool| TextFormat {
        font_id: font.clone(),
        color: col,
        italics,
        ..Default::default()
    };
    let (line_comment, keywords) = match lang {
        Lang::Python => ("#", PY_KEYWORDS),
        Lang::Pine => ("//", PINE_KEYWORDS),
        Lang::Luau => ("--", LUAU_KEYWORDS),
        Lang::Js | Lang::Ts => ("//", JS_KEYWORDS),
    };
    let consts = constants(lang);
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    let mut plain = String::new();
    let flush = |job: &mut LayoutJob, plain: &mut String| {
        if !plain.is_empty() {
            job.append(plain, 0.0, fmt(c.text, false));
            plain.clear();
        }
    };
    let take = |a: usize, b: usize| chars[a..b].iter().collect::<String>();
    while i < chars.len() {
        let ch = chars[i];
        let rest: String = chars[i..chars.len().min(i + 2)].iter().collect();
        if rest.starts_with(line_comment) {
            flush(&mut job, &mut plain);
            let end = chars[i..]
                .iter()
                .position(|c| *c == '\n')
                .map(|p| i + p)
                .unwrap_or(chars.len());
            job.append(&take(i, end), 0.0, fmt(c.comment, true));
            i = end;
        } else if ch == '"' || ch == '\'' || (ch == '`' && matches!(lang, Lang::Js | Lang::Ts)) {
            flush(&mut job, &mut plain);
            let mut j = i + 1;
            while j < chars.len() && chars[j] != ch && (chars[j] != '\n' || ch == '`') {
                if chars[j] == '\\' {
                    j += 1;
                }
                j += 1;
            }
            let end = (j + 1).min(chars.len());
            job.append(&take(i, end), 0.0, fmt(c.string, false));
            i = end;
        } else if ch.is_ascii_digit() && (i == 0 || !(chars[i - 1].is_alphanumeric() || chars[i - 1] == '_')) {
            flush(&mut job, &mut plain);
            let mut j = i;
            while j < chars.len() && (chars[j].is_ascii_alphanumeric() || chars[j] == '.' || chars[j] == '_') {
                j += 1;
            }
            job.append(&take(i, j), 0.0, fmt(c.number, false));
            i = j;
        } else if ch.is_alphabetic() || ch == '_' {
            let mut j = i;
            while j < chars.len() && (chars[j].is_alphanumeric() || chars[j] == '_') {
                j += 1;
            }
            let word = take(i, j);
            let next = chars[j..].iter().find(|c| **c != ' ').copied();
            let col = if keywords.contains(&word.as_str()) {
                Some((c.keyword, false))
            } else if consts.contains(&word.as_str()) {
                Some((c.number, false))
            } else if chars.get(j) == Some(&'.') {
                Some((c.ns, false))
            } else if next == Some('(') {
                Some((c.func, false))
            } else {
                None
            };
            match col {
                Some((col, it)) => {
                    flush(&mut job, &mut plain);
                    job.append(&word, 0.0, fmt(col, it));
                }
                None => plain.push_str(&word),
            }
            i = j;
        } else if "+-*/%=<>!&|^~?:".contains(ch) {
            flush(&mut job, &mut plain);
            let mut j = i;
            while j < chars.len()
                && "+-*/%=<>!&|^~?:".contains(chars[j])
                && !take(j, (j + 2).min(chars.len())).starts_with(line_comment)
            {
                j += 1;
            }
            let j = j.max(i + 1);
            job.append(&take(i, j), 0.0, fmt(c.op, false));
            i = j;
        } else if "()[]{},;.".contains(ch) {
            flush(&mut job, &mut plain);
            job.append(&ch.to_string(), 0.0, fmt(c.punct, false));
            i += 1;
        } else {
            plain.push(ch);
            i += 1;
        }
    }
    flush(&mut job, &mut plain);
    job
}

/// Each language's icon and colour, for the explorer, tabs and menus.
fn file_icon(lang: Lang) -> (&'static str, Color32) {
    match lang {
        Lang::Pine => (ph::TREE_EVERGREEN, Color32::from_rgb(0x4C, 0xB8, 0x7A)),
        Lang::Python => (ph::FILE_PY, Color32::from_rgb(0x5A, 0x9F, 0xE0)),
        Lang::Luau => (ph::MOON, Color32::from_rgb(0x7A, 0xA2, 0xF7)),
        Lang::Js => (ph::FILE_JS, Color32::from_rgb(0xE5, 0xC0, 0x4B)),
        Lang::Ts => (ph::FILE_TS, Color32::from_rgb(0x3C, 0x8C, 0xE7)),
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

/// "ideas/rsi.pine" → "ideas" ("" at the top).
fn parent_dir(path: &str) -> &str {
    path.rsplit_once('/').map(|(d, _)| d).unwrap_or("")
}

/// "ideas/rsi.pine" → "rsi.pine".
fn base_name(path: &str) -> &str {
    path.rsplit_once('/').map(|(_, n)| n).unwrap_or(path)
}

fn join_path(dir: &str, name: &str) -> String {
    if dir.is_empty() {
        name.to_string()
    } else {
        format!("{dir}/{name}")
    }
}

/// A row of the explorer's tree.
enum Node {
    Dir {
        path: String,
        depth: usize,
        open: bool,
        count: usize,
    },
    File {
        path: String,
        depth: usize,
    },
}

/// The explorer's rows: each folder's subfolders, then its scripts; a
/// closed folder's contents left out.
fn tree(files: &[String], dirs: &[String], open: &std::collections::HashSet<String>) -> Vec<Node> {
    fn level(
        at: &str,
        depth: usize,
        files: &[String],
        dirs: &[String],
        open: &std::collections::HashSet<String>,
        out: &mut Vec<Node>,
    ) {
        for d in dirs.iter().filter(|d| parent_dir(d) == at) {
            let is_open = open.contains(d);
            let count = files.iter().filter(|f| f.starts_with(&format!("{d}/"))).count();
            out.push(Node::Dir {
                path: d.clone(),
                depth,
                open: is_open,
                count,
            });
            if is_open {
                level(d, depth + 1, files, dirs, open, out);
            }
        }
        for f in files.iter().filter(|f| parent_dir(f) == at) {
            out.push(Node::File { path: f.clone(), depth });
        }
    }
    let mut out = Vec::new();
    level("", 0, files, dirs, open, &mut out);
    out
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
            title: base_name(file).to_string(),
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

    /// A new script from its language's template, in the folder last
    /// picked in the explorer; its name is ready to type.
    fn new_script(&mut self, lang: Lang) {
        let dir = self.editor.new_dir.clone();
        let _ = std::fs::create_dir_all(self.scripts_dir().join(&dir));
        let name = self.free_name(&dir, "Untitled", lang.ext());
        let body = match lang {
            Lang::Python => PYTHON_TEMPLATE,
            Lang::Pine => PINE_TEMPLATE,
            Lang::Luau => LUAU_TEMPLATE,
            Lang::Js => JS_TEMPLATE,
            Lang::Ts => TS_TEMPLATE,
        };
        if std::fs::write(self.scripts_dir().join(&name), body).is_ok() {
            self.open_in_editor(&name);
            self.editor.renaming = Some((name.clone(), base_name(&name).to_string()));
        }
    }

    /// A free "<stem>.ext" (or "<stem> 2.ext"…) in folder `dir`, as a path.
    fn free_name(&self, dir: &str, stem: &str, ext: &str) -> String {
        let root = self.scripts_dir();
        std::iter::once(format!("{stem}{ext}"))
            .chain((2..).map(|k| format!("{stem} {k}{ext}")))
            .map(|n| join_path(dir, &n))
            .find(|p| !root.join(p).exists())
            .expect("a free name")
    }

    /// Copies a read-only built-in source into the scripts folder.
    fn copy_builtin(&mut self) {
        let Some(d) = self.editor.doc() else { return };
        let (stem, text) = (d.title.clone(), d.text.clone());
        let dir = self.editor.new_dir.clone();
        let name = self.free_name(&dir, &stem, ".pine");
        let _ = std::fs::create_dir_all(self.scripts_dir().join(&dir));
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
        let stem = base_name(file).strip_suffix(lang.ext()).unwrap_or(base_name(file));
        let name = self.free_name(parent_dir(file), &format!("{stem} copy"), lang.ext());
        let root = self.scripts_dir();
        if std::fs::copy(root.join(file), root.join(&name)).is_ok() {
            self.open_in_editor(&name);
        }
    }

    /// Whether a script in `path` (a file, or anything in a folder) is on
    /// the chart: those keep their place until taken off it.
    fn on_chart_under(&self, path: &str) -> bool {
        self.scripts
            .iter()
            .any(|r| r.file == path || r.file.starts_with(&format!("{path}/")))
    }

    /// Open tabs, and the explorer's state, follow a file or folder that
    /// moved from `from` to `to`.
    fn follow_move(&mut self, from: &str, to: &str) {
        let moved = |k: &str| -> Option<String> {
            if k == from {
                Some(to.to_string())
            } else {
                k.strip_prefix(&format!("{from}/")).map(|rest| format!("{to}/{rest}"))
            }
        };
        for d in self.editor.docs.iter_mut().filter(|d| !d.readonly) {
            if let Some(k) = moved(&d.key) {
                d.title = base_name(&k).to_string();
                d.lang = Lang::of(&k).unwrap_or(d.lang);
                d.key = k;
            }
        }
        let open: Vec<String> = self.editor.open_dirs.iter().cloned().collect();
        self.editor.open_dirs = open.into_iter().map(|d| moved(&d).unwrap_or(d)).collect();
        if let Some(k) = moved(&self.editor.new_dir) {
            self.editor.new_dir = k;
        }
    }

    /// Renames a file or folder in place (`to` is the new name only).
    fn rename_path(&mut self, from: &str, to: &str) {
        let to = to.trim();
        let root = self.scripts_dir();
        let is_dir = root.join(from).is_dir();
        if to.is_empty() || to.contains(['/', '\\']) || to.starts_with('.') {
            return;
        }
        // A script keeps its language: "My RSI" becomes "My RSI.pine".
        let name = match Lang::of(from) {
            Some(lang) if !is_dir && Lang::of(to).is_none() => format!("{to}{}", lang.ext()),
            _ => to.to_string(),
        };
        let dest = join_path(parent_dir(from), &name);
        if dest == from {
            return;
        }
        if self.on_chart_under(from) {
            self.editor.status = "Take its scripts off the chart before renaming it.".into();
            return;
        }
        if root.join(&dest).exists() {
            self.editor.status = format!("There's already something called {name} there.");
            return;
        }
        if let Err(e) = std::fs::rename(root.join(from), root.join(&dest)) {
            self.editor.status = format!("Couldn't rename it: {e}");
            return;
        }
        self.follow_move(from, &dest);
    }

    /// Moves a script into folder `dir` ("" is the scripts folder itself).
    fn move_script(&mut self, file: &str, dir: &str) {
        let dest = join_path(dir, base_name(file));
        if dest == file || parent_dir(file) == dir {
            return;
        }
        if self.on_chart_under(file) {
            self.editor.status = "Take it off the chart before moving it.".into();
            return;
        }
        let root = self.scripts_dir();
        if root.join(&dest).exists() {
            self.editor.status = format!("{dir} already has a {}.", base_name(file));
            return;
        }
        match std::fs::rename(root.join(file), root.join(&dest)) {
            Ok(()) => {
                self.follow_move(file, &dest);
                if !dir.is_empty() {
                    self.editor.open_dirs.insert(dir.to_string());
                }
                self.editor.status = format!("Moved to {}.", if dir.is_empty() { "scripts" } else { dir });
            }
            Err(e) => self.editor.status = format!("Couldn't move it: {e}"),
        }
    }

    /// A new folder in `dir`, with its name ready to type.
    fn new_folder(&mut self, dir: &str) {
        let root = self.scripts_dir();
        let path = std::iter::once("New folder".to_string())
            .chain((2..).map(|k| format!("New folder {k}")))
            .map(|n| join_path(dir, &n))
            .find(|p| !root.join(p).exists())
            .expect("a free name");
        match std::fs::create_dir_all(root.join(&path)) {
            Ok(()) => {
                if !dir.is_empty() {
                    self.editor.open_dirs.insert(dir.to_string());
                }
                self.editor.scripts_open = true;
                self.editor.renaming = Some((path.clone(), base_name(&path).to_string()));
            }
            Err(e) => self.editor.status = format!("Couldn't make the folder: {e}"),
        }
    }

    /// Deletes a folder, only when it's empty (scripts are never deleted
    /// in bulk).
    fn delete_folder(&mut self, dir: &str) {
        let path = self.scripts_dir().join(dir);
        let empty = std::fs::read_dir(&path)
            .map(|mut d| d.next().is_none())
            .unwrap_or(false);
        if !empty {
            self.editor.status = format!("{dir} isn't empty: move or delete what's in it first.");
            return;
        }
        match std::fs::remove_dir(&path) {
            Ok(()) => {
                self.editor.open_dirs.remove(dir);
                if self.editor.new_dir == dir {
                    self.editor.new_dir = parent_dir(dir).to_string();
                }
                self.editor.status = format!("Deleted the folder {dir}.");
            }
            Err(e) => self.editor.status = format!("Couldn't delete it: {e}"),
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
        let (run, save, close, side) = ui.input_mut(|i| {
            (
                i.consume_key(Modifiers::COMMAND, Key::Enter),
                i.consume_key(Modifiers::COMMAND, Key::S),
                i.consume_key(Modifiers::COMMAND, Key::W),
                i.consume_key(Modifiers::COMMAND, Key::B),
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
        if side {
            self.editor.sidebar = !self.editor.sidebar;
        }

        let full = ui.max_rect();
        let c = code_colors(t);
        ui.painter().rect_filled(full, CornerRadius::ZERO, c.bg);
        let status = Rect::from_min_max(pos2(full.left(), full.bottom() - STATUS_H), full.max);
        let work = Rect::from_min_max(full.min, pos2(full.right(), status.top()));
        let ew = if self.editor.sidebar {
            self.editor.explorer_w.clamp(170.0, (work.width() * 0.45).max(170.0))
        } else {
            0.0
        };
        let main = Rect::from_min_max(pos2(work.left() + ew, work.top()), work.max);
        if self.editor.sidebar {
            let side = Rect::from_min_size(work.min, vec2(ew, work.height()));
            ui.painter().rect_filled(side, CornerRadius::ZERO, c.side);
            ui.painter()
                .vline(side.right() - 0.5, side.y_range(), Stroke::new(1.0, c.line));
            let d = crate::widgets::drag_handle(
                ui,
                t,
                Rect::from_min_max(
                    pos2(side.right() - 3.0, side.top()),
                    pos2(side.right() + 3.0, side.bottom()),
                ),
                "explorer-resize",
                true,
            );
            if d != 0.0 {
                self.editor.explorer_w = (ew + d).clamp(170.0, 460.0);
            }
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(side));
            child.set_clip_rect(side.intersect(ui.clip_rect()));
            self.explorer(&mut child, t, &files);
        }
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(main));
        child.set_clip_rect(main.intersect(ui.clip_rect()));
        self.editor_main(&mut child, t);
        self.status_bar(ui, t, status);
    }

    fn explorer(&mut self, ui: &mut Ui, t: &Theme, files: &[String]) {
        use crate::widgets as w;
        let c = code_colors(t);
        let full = ui.max_rect();
        // Header: EXPLORER and its actions.
        let head = Rect::from_min_size(full.min, vec2(full.width(), 36.0));
        ui.painter().text(
            pos2(head.left() + 16.0, head.center().y),
            Align2::LEFT_CENTER,
            "EXPLORER",
            theme::medium(11.0),
            c.dim,
        );
        let mut bar = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(head.shrink2(vec2(8.0, 5.0)))
                .layout(egui::Layout::right_to_left(egui::Align::Center)),
        );
        bar.spacing_mut().item_spacing.x = 0.0;
        let more = w::tool_button(&mut bar, t, ph::DOTS_THREE, "More", false, 24.0);
        egui::Popup::menu(&more).show(|ui| {
            ui.set_width(220.0);
            ui.spacing_mut().item_spacing.y = 1.0;
            if w::menu_item(ui, t, Some(ph::FOLDER_OPEN), "Open the scripts folder", "", false).clicked() {
                let dir = self.scripts_dir();
                let _ = std::fs::create_dir_all(&dir);
                crate::app::open_path(&dir);
                ui.close();
            }
            if w::menu_item(
                ui,
                t,
                Some(ph::SIDEBAR_SIMPLE),
                "Hide the sidebar",
                shortcut!("B"),
                false,
            )
            .clicked()
            {
                self.editor.sidebar = false;
                ui.close();
            }
        });
        let find = self.editor.filtering;
        if w::tool_button(&mut bar, t, ph::MAGNIFYING_GLASS, "Filter files", find, 24.0).clicked() {
            self.editor.filtering = !find;
            self.editor.filter.clear();
        }
        if w::tool_button(&mut bar, t, ph::FOLDER_PLUS, "New folder", false, 24.0).clicked() {
            let dir = self.editor.new_dir.clone();
            self.new_folder(&dir);
        }
        let add = w::tool_button(&mut bar, t, ph::FILE_PLUS, "New script", false, 24.0);
        egui::Popup::menu(&add).show(|ui| {
            ui.set_width(210.0);
            ui.spacing_mut().item_spacing.y = 1.0;
            for lang in [Lang::Pine, Lang::Python, Lang::Luau, Lang::Js, Lang::Ts] {
                let (icon, _) = file_icon(lang);
                if w::menu_item(ui, t, Some(icon), lang_name(lang), lang.ext(), false).clicked() {
                    self.new_script(lang);
                    ui.close();
                }
            }
        });

        let mut y = head.bottom();
        if self.editor.filtering {
            let r = Rect::from_min_size(pos2(full.left() + 10.0, y), vec2(full.width() - 20.0, 34.0));
            let mut f = ui.new_child(egui::UiBuilder::new().max_rect(r));
            let resp = w::search_field(&mut f, t, &mut self.editor.filter, "Filter", r.width());
            if !resp.has_focus() && self.editor.filter.is_empty() && f.input(|i| i.key_pressed(Key::Escape)) {
                self.editor.filtering = false;
            }
            y += 38.0;
        }

        // The outline sits at the bottom, the files above it.
        let outline_items = self.editor.doc().map(|d| outline(&d.text, d.lang)).unwrap_or_default();
        let outline_h = if self.editor.outline_open {
            (24.0 + 22.0 * outline_items.len().max(1) as f32).min(full.height() * 0.4)
        } else {
            24.0
        };
        let files_rect = Rect::from_min_max(pos2(full.left(), y), pos2(full.right(), full.bottom() - outline_h));
        let outline_rect = Rect::from_min_max(pos2(full.left(), files_rect.bottom()), full.max);

        let q = self.editor.filter.trim().to_lowercase();
        let active_key = self.editor.doc().map(|d| d.key.clone());
        let mut open = None;
        let mut action: Option<(u8, String)> = None;
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(files_rect));
        child.set_clip_rect(files_rect.intersect(ui.clip_rect()));
        egui::ScrollArea::vertical()
            .id_salt("explorer")
            .auto_shrink([false; 2])
            .show(&mut child, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                // The scripts folder as a tree; with a filter, matching
                // scripts as a flat list of paths.
                let dirs = self.script_dirs();
                let root_open = self.editor.scripts_open || !q.is_empty();
                let head = folder_row(ui, t, "SCRIPTS", root_open, files.len());
                if head.clicked() {
                    self.editor.scripts_open = !self.editor.scripts_open;
                    self.editor.new_dir.clear();
                }
                drop_target(ui, t, &head, "", &mut action);
                head.context_menu(|ui| folder_menu(ui, t, "", &mut action));
                if root_open {
                    if files.is_empty() && dirs.is_empty() {
                        ui.horizontal(|ui| {
                            ui.add_space(30.0);
                            ui.label(
                                egui::RichText::new("No scripts yet. Press + to start one.")
                                    .font(theme::regular(12.0))
                                    .color(c.dim),
                            );
                        });
                    }
                    let rows: Vec<Node> = if q.is_empty() {
                        tree(files, &dirs, &self.editor.open_dirs)
                    } else {
                        files
                            .iter()
                            .filter(|f| f.to_lowercase().contains(&q))
                            .map(|f| Node::File {
                                path: f.clone(),
                                depth: 0,
                            })
                            .collect()
                    };
                    for row in rows {
                        let (path, depth) = match &row {
                            Node::Dir { path, depth, .. } | Node::File { path, depth } => (path.clone(), *depth),
                        };
                        // Being renamed: a field in the row's place.
                        if let Some((from, to)) = &mut self.editor.renaming
                            && *from == path
                        {
                            ui.horizontal(|ui| {
                                ui.add_space(26.0 + depth as f32 * 14.0);
                                let r = ui.add(
                                    w::field(to)
                                        .desired_width(ui.available_width() - 12.0)
                                        .font(theme::regular(12.5))
                                        .margin(vec2(8.0, 3.0)),
                                );
                                if !r.has_focus() && !r.lost_focus() {
                                    r.request_focus();
                                }
                                let (enter, esc) =
                                    ui.input(|i| (i.key_pressed(Key::Enter), i.key_pressed(Key::Escape)));
                                if r.lost_focus() || enter || esc {
                                    action = Some(if esc { (9, String::new()) } else { (3, to.clone()) });
                                }
                            });
                            continue;
                        }
                        match row {
                            Node::Dir {
                                path,
                                depth,
                                open,
                                count,
                            } => {
                                let picked = self.editor.new_dir == path;
                                let resp = dir_row(ui, t, base_name(&path), open, depth + 1, count, picked);
                                if resp.clicked() {
                                    action = Some((15, path.clone()));
                                }
                                if resp.double_clicked() {
                                    action = Some((4, path.clone()));
                                }
                                drop_target(ui, t, &resp, &path, &mut action);
                                resp.context_menu(|ui| folder_menu(ui, t, &path, &mut action));
                            }
                            Node::File { path: f, depth } => {
                                let lang = Lang::of(&f).unwrap_or(Lang::Pine);
                                let on_chart = self.scripts.iter().any(|r| r.file == f);
                                let selected = active_key.as_deref() == Some(f.as_str());
                                let dirty = self.editor.docs.iter().any(|d| d.key == f && d.dirty());
                                // Filtered: the path, so same names in two folders read apart.
                                let label = if q.is_empty() { base_name(&f) } else { f.as_str() };
                                let resp = file_row(ui, t, lang, label, selected, on_chart, dirty, depth + 1);
                                if resp.drag_started() {
                                    resp.dnd_set_drag_payload(f.clone());
                                }
                                if resp.clicked() {
                                    open = Some(f.clone());
                                }
                                if resp.double_clicked() {
                                    action = Some((4, f.clone()));
                                }
                                resp.context_menu(|ui| {
                                    ui.set_width(210.0);
                                    ui.spacing_mut().item_spacing.y = 1.0;
                                    let mut item = |ui: &mut Ui, icon, label: &str, what: u8| {
                                        if w::menu_item(ui, t, Some(icon), label, "", false).clicked() {
                                            action = Some((what, f.clone()));
                                            ui.close();
                                        }
                                    };
                                    item(
                                        ui,
                                        ph::PLAY,
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
                        }
                    }
                }
                // Built-ins' Pine source, read-only.
                let list: Vec<(&str, &str)> = crate::sources::SOURCES
                    .iter()
                    .filter_map(|(id, _)| opentern_series::builtins::by_id(id).map(|m| (*id, m.name)))
                    .filter(|(_, n)| q.is_empty() || n.to_lowercase().contains(&q))
                    .collect();
                let open_list = self.editor.show_builtins || !q.is_empty();
                if folder_row(ui, t, "BUILT-IN INDICATORS", open_list, list.len()).clicked() {
                    self.editor.show_builtins = !self.editor.show_builtins;
                }
                if open_list {
                    for (id, name) in list {
                        let key = format!("builtin:{id}");
                        let selected = active_key.as_deref() == Some(key.as_str());
                        if file_row(ui, t, Lang::Pine, name, selected, false, false, 1).clicked() {
                            open = Some(key);
                        }
                    }
                }
                ui.add_space(8.0);
            });

        // Outline: what's defined in the open file; a click goes there.
        ui.painter().hline(
            outline_rect.x_range(),
            outline_rect.top() + 0.5,
            Stroke::new(1.0, c.line),
        );
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(outline_rect));
        child.set_clip_rect(outline_rect.intersect(ui.clip_rect()));
        child.spacing_mut().item_spacing.y = 0.0;
        if folder_row(&mut child, t, "OUTLINE", self.editor.outline_open, outline_items.len()).clicked() {
            self.editor.outline_open = !self.editor.outline_open;
        }
        let mut jump = None;
        if self.editor.outline_open {
            egui::ScrollArea::vertical()
                .id_salt("outline")
                .auto_shrink([false; 2])
                .show(&mut child, |ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    if outline_items.is_empty() {
                        ui.horizontal(|ui| {
                            ui.add_space(30.0);
                            ui.label(
                                egui::RichText::new("Nothing defined yet.")
                                    .font(theme::regular(12.0))
                                    .color(c.dim),
                            );
                        });
                    }
                    for (line, name, kind) in &outline_items {
                        let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::click());
                        if ui.rect_contains_pointer(r) {
                            ui.painter().rect_filled(r, CornerRadius::ZERO, t.hover_wash());
                        }
                        let (icon, col) = match kind {
                            Outline::Function => (ph::CUBE, c.keyword),
                            Outline::Input => (ph::SLIDERS_HORIZONTAL, c.ns),
                            Outline::Type => (ph::BRACKETS_CURLY, c.number),
                            Outline::Variable => (ph::CUBE_TRANSPARENT, c.func),
                        };
                        let p = ui.painter();
                        p.text(
                            pos2(r.left() + 30.0, r.center().y),
                            Align2::CENTER_CENTER,
                            icon,
                            theme::regular(12.5),
                            col,
                        );
                        p.text(
                            pos2(r.left() + 44.0, r.center().y),
                            Align2::LEFT_CENTER,
                            name,
                            theme::regular(12.5),
                            c.text,
                        );
                        p.text(
                            pos2(r.right() - 12.0, r.center().y),
                            Align2::RIGHT_CENTER,
                            (line + 1).to_string(),
                            FontId::monospace(11.0),
                            c.dim,
                        );
                        if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                            jump = Some(*line);
                        }
                    }
                });
        }
        if let (Some(line), Some(d)) = (jump, self.editor.docs.get_mut(self.editor.active)) {
            d.jump = Some(line);
        }

        if let Some(k) = open {
            match k.strip_prefix("builtin:") {
                Some(id) => self.open_builtin_source(id),
                None => {
                    // New scripts go next to the one opened.
                    self.editor.new_dir = parent_dir(&k).to_string();
                    self.open_in_editor(&k);
                }
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
                    self.rename_path(&from, &to);
                }
            }
            Some((4, f)) => {
                let name = base_name(&f).to_string();
                self.editor.renaming = Some((f, name));
            }
            // Folders: a new folder or script in one, delete, open in the
            // file manager, open/close, and a script dropped on one.
            Some((10, dir)) => self.new_folder(&dir),
            Some((11, dir)) => self.delete_folder(&dir),
            Some((12, dir)) => {
                let path = self.scripts_dir().join(&dir);
                let _ = std::fs::create_dir_all(&path);
                crate::app::open_path(&path);
            }
            Some((13, both)) => {
                if let Some((file, dir)) = both.split_once('\0') {
                    self.move_script(file, dir);
                }
            }
            Some((14, both)) => {
                if let Some((dir, ext)) = both.split_once('\0')
                    && let Some(lang) = Lang::of(&format!("x{ext}"))
                {
                    self.editor.new_dir = dir.to_string();
                    if !dir.is_empty() {
                        self.editor.open_dirs.insert(dir.to_string());
                    }
                    self.new_script(lang);
                }
            }
            Some((15, dir)) => {
                if !self.editor.open_dirs.remove(&dir) {
                    self.editor.open_dirs.insert(dir.clone());
                }
                self.editor.new_dir = dir;
            }
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
        let c = code_colors(t);
        let full = ui.max_rect();
        // Tabs, with the run and more buttons at the end.
        let bar = Rect::from_min_size(full.min, vec2(full.width(), TAB_H));
        ui.painter().rect_filled(bar, CornerRadius::ZERO, c.side);
        let tools_w = 96.0;
        let tabs_rect = Rect::from_min_max(bar.min, pos2(bar.right() - tools_w, bar.bottom()));
        if !self.editor.sidebar {
            // A way back to the files.
            let r = Rect::from_min_size(bar.min + vec2(4.0, 4.0), vec2(28.0, 28.0));
            let mut b = ui.new_child(egui::UiBuilder::new().max_rect(r));
            if crate::widgets::tool_button(
                &mut b,
                t,
                ph::SIDEBAR_SIMPLE,
                concat!("Show files (", shortcut!("B"), ")"),
                false,
                28.0,
            )
            .clicked()
            {
                self.editor.sidebar = true;
            }
            self.doc_tabs(
                ui,
                t,
                Rect::from_min_max(pos2(tabs_rect.left() + 36.0, bar.top()), tabs_rect.max),
            );
        } else {
            self.doc_tabs(ui, t, tabs_rect);
        }
        let mut tools = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(Rect::from_min_max(
                    pos2(tabs_rect.right(), bar.top()),
                    bar.max - vec2(6.0, 0.0),
                ))
                .layout(egui::Layout::right_to_left(egui::Align::Center)),
        );
        self.editor_tools(&mut tools, t);
        ui.painter()
            .hline(bar.x_range(), bar.bottom() - 0.5, Stroke::new(1.0, c.line));

        if self.editor.docs.is_empty() {
            let rest = Rect::from_min_max(pos2(full.left(), bar.bottom()), full.max);
            let mut e = ui.new_child(egui::UiBuilder::new().max_rect(rest.shrink(20.0)));
            crate::widgets::empty_state(&mut e, t, ph::CODE, "No file open");
            return;
        }

        // Breadcrumbs: folder › file › the definition the cursor is in.
        let crumbs = Rect::from_min_size(pos2(full.left(), bar.bottom()), vec2(full.width(), CRUMB_H));
        self.breadcrumbs(ui, t, crumbs);

        // Problems / Output under the code.
        let panel_h = if self.editor.panel_open { PANEL_H } else { 28.0 };
        let panel = Rect::from_min_max(pos2(full.left(), full.bottom() - panel_h), full.max);
        let code = Rect::from_min_max(pos2(full.left(), crumbs.bottom()), pos2(full.right(), panel.top()));
        if code.height() > 30.0 {
            self.code_view(ui, t, code);
        }
        self.problems(ui, t, panel);
    }

    fn breadcrumbs(&self, ui: &mut Ui, t: &Theme, r: Rect) {
        let c = code_colors(t);
        let Some(d) = self.editor.doc() else { return };
        let p = ui.painter();
        let font = theme::regular(12.0);
        let y = r.center().y;
        let mut x = r.left() + 16.0;
        let mut put = |p: &egui::Painter, text: &str, col: Color32, font: &FontId| {
            let g = p.text(pos2(x, y), Align2::LEFT_CENTER, text, font.clone(), col);
            x = g.right() + 6.0;
        };
        put(p, if d.readonly { "built-in" } else { "scripts" }, c.dim, &font);
        put(p, ph::CARET_RIGHT, c.dim, &theme::regular(10.0));
        // The folders it's in.
        if !d.readonly {
            for part in parent_dir(&d.key).split('/').filter(|x| !x.is_empty()) {
                put(p, ph::FOLDER_SIMPLE, c.dim, &theme::regular(12.0));
                put(p, part, c.dim, &font);
                put(p, ph::CARET_RIGHT, c.dim, &theme::regular(10.0));
            }
        }
        let (icon, col) = file_icon(d.lang);
        put(p, icon, col, &theme::regular(13.0));
        put(p, &d.title, c.text2, &font);
        let line = self.editor.caret.0.saturating_sub(1);
        if let Some((_, name, _)) = outline(&d.text, d.lang).into_iter().rev().find(|(l, _, _)| *l <= line) {
            put(p, ph::CARET_RIGHT, c.dim, &theme::regular(10.0));
            put(p, ph::CUBE, c.keyword, &theme::regular(12.0));
            put(p, &name, c.text2, &font);
        }
    }

    fn doc_tabs(&mut self, ui: &mut Ui, t: &Theme, rect: Rect) {
        let c = code_colors(t);
        let font = theme::regular(12.5);
        let n = self.editor.docs.len();
        if n == 0 {
            return;
        }
        // Each tab: [pad][icon][gap][name][gap][close][pad]; names shrink
        // (with "…") when the tabs don't fit.
        let fixed = 12.0 + 16.0 + 7.0 + 10.0 + 20.0 + 6.0;
        let room = ((rect.width() / n as f32) - fixed).clamp(36.0, 170.0);
        let mut x = rect.left();
        let mut pick = None;
        let mut close = None;
        for (i, d) in self.editor.docs.iter().enumerate() {
            let p = ui.painter();
            let label = if d.readonly {
                format!("{}  {}", d.title, ph::LOCK_SIMPLE)
            } else {
                d.title.clone()
            };
            let name = crate::format::elide_to(p, &label, &font, room);
            let name_w = p.layout_no_wrap(name.clone(), font.clone(), c.text).size().x;
            let w = fixed + name_w;
            if x + w > rect.right() + 0.5 {
                break;
            }
            let r = Rect::from_min_size(pos2(x, rect.top()), vec2(w, rect.height()));
            x += w;
            let resp = ui.interact(r, Id::new(("doc-tab", &d.key)), Sense::click());
            let on = i == self.editor.active;
            let inside = ui.rect_contains_pointer(r);
            let p = ui.painter().with_clip_rect(r);
            if on {
                p.rect_filled(r, CornerRadius::ZERO, c.bg);
                p.hline(r.x_range(), r.top() + 1.0, Stroke::new(2.0, t.accent));
            } else if inside {
                p.rect_filled(r, CornerRadius::ZERO, t.hover_wash().gamma_multiply(0.6));
            }
            p.vline(r.right() - 0.5, r.y_range(), Stroke::new(1.0, c.line));
            let cy = r.center().y;
            let (icon, col) = file_icon(d.lang);
            p.text(
                pos2(r.left() + 20.0, cy),
                Align2::CENTER_CENTER,
                icon,
                theme::regular(14.0),
                col,
            );
            p.text(
                pos2(r.left() + 12.0 + 16.0 + 7.0, cy),
                Align2::LEFT_CENTER,
                name,
                font.clone(),
                if on { c.text } else { c.text2 },
            );
            let xr = Rect::from_center_size(pos2(r.right() - 16.0, cy), vec2(20.0, 20.0));
            let xresp = ui.interact(xr, Id::new(("doc-x", &d.key)), Sense::click());
            if d.dirty() && !inside {
                p.circle_filled(xr.center(), 3.5, c.text2);
            } else if inside || on {
                if xresp.hovered() {
                    p.rect_filled(xr, CornerRadius::same(4), t.hover_wash());
                }
                p.text(xr.center(), Align2::CENTER_CENTER, ph::X, theme::regular(12.0), c.text2);
            }
            if xresp.on_hover_text(concat!("Close (", shortcut!("W"), ")")).clicked() || resp.middle_clicked() {
                close = Some(i);
            } else if resp.on_hover_text(&d.title).clicked() {
                pick = Some(i);
            }
        }
        if let Some(i) = pick {
            self.editor.active = i;
        }
        if let Some(i) = close {
            let d = &self.editor.docs[i];
            if d.dirty() && self.editor.deleting.as_deref() != Some(d.key.as_str()) {
                self.editor.status = format!("{} has unsaved changes: close again to discard them.", d.title);
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
        ui.spacing_mut().item_spacing.x = 0.0;
        let (file, readonly) = (d.key.clone(), d.readonly);
        let on_chart = self.scripts.iter().any(|r| r.file == file);
        let more = w::tool_button(ui, t, ph::DOTS_THREE, "More", false, 28.0);
        egui::Popup::menu(&more).show(|ui| {
            ui.set_width(230.0);
            ui.spacing_mut().item_spacing.y = 1.0;
            if readonly {
                if w::menu_item(ui, t, Some(ph::COPY), "Make a copy to edit", "", false).clicked() {
                    self.copy_builtin();
                    ui.close();
                }
                return;
            }
            if w::menu_item(ui, t, Some(ph::FLOPPY_DISK), "Save", shortcut!("S"), false).clicked() {
                self.save_script();
                ui.close();
            }
            if w::menu_item(ui, t, Some(ph::ARROW_SQUARE_OUT), "Open in your editor", "", false)
                .on_hover_text("The chart reloads when you save there")
                .clicked()
            {
                crate::app::open_path(&self.scripts_dir().join(&file));
                ui.close();
            }
            if w::menu_item(ui, t, Some(ph::FOLDER_OPEN), "Open the scripts folder", "", false).clicked() {
                crate::app::open_path(&self.scripts_dir());
                ui.close();
            }
        });
        if readonly {
            if w::tool_button(ui, t, ph::COPY, "Make a copy to edit", false, 28.0).clicked() {
                self.copy_builtin();
            }
            return;
        }
        let tip = if on_chart {
            concat!("Save and update the chart (", shortcut!("Enter"), ")")
        } else {
            concat!("Save and add to the chart (", shortcut!("Enter"), ")")
        };
        // The run button, green like an IDE's.
        let (r, resp) = ui.allocate_exact_size(vec2(28.0, 28.0), Sense::click());
        let hov = motion::toggle(ui.ctx(), resp.id, resp.hovered(), motion::MICRO);
        ui.painter()
            .rect_filled(r, CornerRadius::same(7), motion::with_alpha(t.hover_wash(), hov));
        ui.painter()
            .text(r.center(), Align2::CENTER_CENTER, ph::PLAY, theme::regular(16.0), t.pos);
        if resp
            .on_hover_text(tip)
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .clicked()
        {
            self.run_script();
        }
    }

    fn code_view(&mut self, ui: &mut Ui, t: &Theme, rect: Rect) {
        let a = self.editor.active;
        let Some(doc) = self.editor.docs.get(a) else { return };
        let c = code_colors(t);
        let key = doc.key.clone();
        let lang = doc.lang;
        let readonly = doc.readonly;
        let err_line = self
            .scripts
            .iter()
            .find(|r| r.file == key)
            .and_then(|r| r.error.as_deref())
            .and_then(|e| error_line(e, &key));
        let id = Id::new(("code", &key));
        let font = FontId::monospace(CODE_SIZE);
        let row_h = ui.fonts_mut(|f| f.row_height(&font));
        let char_w = ui.fonts_mut(|f| f.glyph_width(&font, 'm'));
        let lines = doc.text.split('\n').count();
        let digits = lines.to_string().len().max(3) as f32;
        let gutter_w = digits * char_w + 34.0;
        let mini_w = if rect.width() > 520.0 { MINIMAP_W } else { 0.0 };
        let text_rect = Rect::from_min_max(rect.min, pos2(rect.right() - mini_w, rect.bottom()));

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

        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(text_rect));
        child.set_clip_rect(text_rect.intersect(ui.clip_rect()));
        let tt = *t;
        let mut layouter = |ui: &Ui, text: &dyn egui::TextBuffer, _wrap: f32| {
            let mut job = highlight(text.as_str(), lang, &tt, FontId::monospace(CODE_SIZE));
            job.wrap.max_width = f32::INFINITY;
            ui.ctx().fonts_mut(|f| f.layout_job(job))
        };
        let unit = indent_unit(lang).len();
        let mut caret = None;
        let mut scroll = egui::ScrollArea::both()
            .id_salt(("code-scroll", &key))
            .auto_shrink([false; 2]);
        if let Some(y) = self.editor.minimap_to.take() {
            scroll = scroll.vertical_scroll_offset(y);
        }
        let out = scroll.show(&mut child, |ui| {
            let view = ui.clip_rect();
            let bg = ui.painter().add(egui::Shape::Noop);
            let fg = ui.painter().add(egui::Shape::Noop);
            ui.horizontal_top(|ui| {
                ui.add_space(gutter_w);
                let width = (view.width() - gutter_w - 8.0).max(100.0);
                let rows = ((view.height() - 12.0) / row_h).max(1.0) as usize;
                fn edit(text: &mut dyn egui::TextBuffer, id: Id, width: f32, rows: usize) -> egui::TextEdit<'_> {
                    egui::TextEdit::multiline(text)
                        .id(id)
                        .frame(egui::Frame::NONE)
                        .code_editor()
                        .lock_focus(true)
                        .desired_width(width)
                        .desired_rows(rows)
                        .margin(vec2(4.0, 8.0))
                }
                let out = if readonly {
                    let mut ro: &str = doc.text.as_str();
                    edit(&mut ro, id, width, rows).layouter(&mut layouter).show(ui)
                } else {
                    edit(&mut doc.text, id, width, rows).layouter(&mut layouter).show(ui)
                };
                let g = &out.galley;
                let origin = out.galley_pos;
                let full_x = view.x_range();
                let mut under = Vec::new();
                let mut over = Vec::new();
                let cursor_line = out.cursor_range.map(|cr| {
                    let ci = cr.primary.index.0;
                    let before: String = doc.text.chars().take(ci).collect();
                    let line = before.matches('\n').count();
                    let col = before.rsplit('\n').next().map(|l| l.chars().count()).unwrap_or(0);
                    caret = Some((line + 1, col + 1));
                    line
                });
                let focused = out.response.response.has_focus();
                let text_lines: Vec<&str> = doc.text.split('\n').collect();
                let mut last_indent = 0usize;
                for (line, row) in g.rows.iter().enumerate() {
                    let y = origin.y + row.pos.y;
                    if y + row_h < view.top() || y > view.bottom() {
                        continue;
                    }
                    let band = Rect::from_x_y_ranges(full_x, y..=y + row_h);
                    let cur = cursor_line == Some(line);
                    if cur && focused {
                        under.push(egui::Shape::rect_filled(band, 0.0, c.current));
                    }
                    let is_err = err_line == Some(line + 1);
                    if is_err {
                        under.push(egui::Shape::rect_filled(band, 0.0, t.neg.gamma_multiply(0.10)));
                        // A squiggle under the line's text.
                        let x0 = origin.x;
                        let x1 = origin.x + row.rect().width().max(char_w * 4.0);
                        let yb = y + row_h - 1.5;
                        let mut pts = Vec::new();
                        let mut x = x0;
                        let mut up = true;
                        while x <= x1 {
                            pts.push(pos2(x, if up { yb - 1.5 } else { yb + 1.0 }));
                            x += 2.5;
                            up = !up;
                        }
                        over.push(egui::Shape::line(pts, Stroke::new(1.0, t.neg)));
                    }
                    // Indent guides, carried across blank lines.
                    let text = text_lines.get(line).copied().unwrap_or("");
                    let lead = text.chars().take_while(|c| *c == ' ').count()
                        + 4 * text.chars().take_while(|c| *c == '\t').count();
                    let indent = if text.trim().is_empty() { last_indent } else { lead };
                    last_indent = indent;
                    for k in 1..=(indent / unit.max(1)) {
                        let gx = origin.x + ((k - 1) * unit) as f32 * char_w + 0.5;
                        if gx > view.left() + gutter_w - 4.0 {
                            under.push(egui::Shape::vline(gx, y..=y + row_h, Stroke::new(1.0, c.guide)));
                        }
                    }
                    let col = if is_err {
                        t.neg
                    } else if cur {
                        c.text
                    } else {
                        c.dim
                    };
                    let num =
                        ui.painter()
                            .layout_no_wrap((line + 1).to_string(), FontId::monospace(CODE_SIZE - 0.5), col);
                    let at = pos2(
                        view.left() + gutter_w - 18.0 - num.size().x,
                        y + (row_h - num.size().y) / 2.0,
                    );
                    over.push(egui::Shape::galley(at, num, col));
                }
                // The gutter stays put while the code scrolls sideways.
                let gutter = Rect::from_min_size(view.min, vec2(gutter_w - 6.0, view.height()));
                over.insert(0, egui::Shape::rect_filled(gutter, 0.0, c.bg));
                ui.painter().set(bg, egui::Shape::Vec(under));
                ui.painter().set(fg, egui::Shape::Vec(over));
                if let Some(line) = ui.data_mut(|d| d.remove_temp::<usize>(id.with("scroll-to")))
                    && let Some(row) = g.rows.get(line)
                {
                    let y = origin.y + row.pos.y;
                    ui.scroll_to_rect(Rect::from_x_y_ranges(full_x, y..=y + row_h), Some(egui::Align::Center));
                }
            });
        });
        if let Some(cr) = caret {
            self.editor.caret = cr;
        }
        if mini_w > 0.0 {
            let mini = Rect::from_min_max(pos2(rect.right() - mini_w, rect.top()), rect.max);
            let offset = out.state.offset.y;
            let view_h = out.inner_rect.height();
            self.minimap(ui, t, mini, row_h, offset, view_h);
        }
    }

    /// The minimap: every line as thin coloured bars, the visible part
    /// shaded; click or drag to scroll there.
    fn minimap(&mut self, ui: &mut Ui, t: &Theme, r: Rect, row_h: f32, offset: f32, view_h: f32) {
        let c = code_colors(t);
        let Some(d) = self.editor.doc() else { return };
        let job = highlight(&d.text, d.lang, t, FontId::monospace(CODE_SIZE));
        let n = d.text.split('\n').count().max(1);
        let lh = (r.height() / n as f32).min(2.6);
        let cw = 1.1;
        let p = ui.painter().with_clip_rect(r);
        p.vline(
            r.left() + 0.5,
            r.y_range(),
            Stroke::new(1.0, c.line.gamma_multiply(0.6)),
        );
        let mut shapes = Vec::new();
        let (mut line, mut col) = (0usize, 0usize);
        for s in &job.sections {
            let piece = &job.text[s.byte_range.start.0..s.byte_range.end.0];
            let colr = s.format.color.gamma_multiply(0.75);
            let mut run: Option<usize> = None;
            for ch in piece.chars().chain(std::iter::once('\0')) {
                let end_run = ch == '\n' || ch == '\0' || ch.is_whitespace();
                if let (true, Some(st)) = (end_run, run) {
                    let y = r.top() + 4.0 + line as f32 * lh;
                    let x0 = r.left() + 8.0 + st as f32 * cw;
                    let x1 = (r.left() + 8.0 + col as f32 * cw).min(r.right() - 4.0);
                    if x1 > x0 {
                        shapes.push(egui::Shape::rect_filled(
                            Rect::from_min_max(pos2(x0, y), pos2(x1, y + (lh * 0.7).max(1.0))),
                            0.0,
                            colr,
                        ));
                    }
                    run = None;
                }
                match ch {
                    '\0' => {}
                    '\n' => {
                        line += 1;
                        col = 0;
                    }
                    c if c.is_whitespace() => col += 1,
                    _ => {
                        run.get_or_insert(col);
                        col += 1;
                    }
                }
            }
        }
        p.extend(shapes);
        // The visible part.
        let first = offset / row_h.max(1.0);
        let shown = view_h / row_h.max(1.0);
        let slider = Rect::from_min_max(
            pos2(r.left() + 1.0, r.top() + 4.0 + first * lh),
            pos2(r.right(), r.top() + 4.0 + (first + shown) * lh),
        );
        let resp = ui.interact(r, Id::new("minimap"), Sense::click_and_drag());
        let hov = resp.hovered() || resp.dragged();
        p.rect_filled(
            slider,
            CornerRadius::ZERO,
            t.text.gamma_multiply(if hov { 0.10 } else { 0.05 }),
        );
        if (resp.clicked() || resp.dragged())
            && let Some(pt) = resp.interact_pointer_pos()
        {
            let line = ((pt.y - r.top() - 4.0) / lh).max(0.0);
            self.editor.minimap_to = Some(((line - shown / 2.0) * row_h).max(0.0));
        }
    }

    /// Problems and Output, under the code (VS Code's bottom panel).
    fn problems(&mut self, ui: &mut Ui, t: &Theme, r: Rect) {
        let c = code_colors(t);
        let Some(d) = self.editor.doc() else { return };
        let (key, readonly, title) = (d.key.clone(), d.readonly, d.title.clone());
        let run = self.scripts.iter().find(|x| x.file == key);
        let on_chart = run.is_some();
        let errors: Vec<String> = run
            .and_then(|x| x.error.clone())
            .map(|e| vec![e.trim().to_string()])
            .unwrap_or_default();
        let log = run.map(|x| x.log.trim().to_string()).unwrap_or_default();
        // A new error opens Problems (once; folding it again sticks).
        let first_err = errors.first().cloned();
        if first_err != self.editor.seen_error {
            if first_err.is_some() {
                self.editor.panel_open = true;
                self.editor.panel_tab = 0;
            }
            self.editor.seen_error = first_err;
        }
        ui.painter().hline(r.x_range(), r.top() + 0.5, Stroke::new(1.0, c.line));
        // Header: the two tabs and a caret to fold.
        let head = Rect::from_min_size(r.min, vec2(r.width(), 28.0));
        let mut x = head.left() + 14.0;
        for (k, label, count) in [(0u8, "PROBLEMS", errors.len()), (1u8, "OUTPUT", 0)] {
            let on = self.editor.panel_tab == k && self.editor.panel_open;
            let g = ui
                .painter()
                .layout_no_wrap(label.to_string(), theme::medium(11.0), c.text);
            let w = g.size().x + if count > 0 { 26.0 } else { 0.0 };
            let tr = Rect::from_min_size(pos2(x, head.top()), vec2(w + 8.0, head.height()));
            let resp = ui.interact(tr, Id::new(("panel-tab", k)), Sense::click());
            let col = if on || resp.hovered() { c.text } else { c.dim };
            ui.painter()
                .galley_with_override_text_color(pos2(x + 4.0, head.center().y - g.size().y / 2.0), g, col);
            if count > 0 {
                let b = Rect::from_center_size(pos2(x + w - 6.0, head.center().y), vec2(18.0, 16.0));
                ui.painter()
                    .rect_filled(b, CornerRadius::same(8), t.neg.gamma_multiply(0.85));
                ui.painter().text(
                    b.center(),
                    Align2::CENTER_CENTER,
                    count.to_string(),
                    theme::semibold(10.5),
                    Color32::WHITE,
                );
            }
            if on {
                ui.painter().hline(
                    tr.shrink2(vec2(4.0, 0.0)).x_range(),
                    head.bottom() - 1.0,
                    Stroke::new(1.5, t.accent),
                );
            }
            if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                if self.editor.panel_open && self.editor.panel_tab == k {
                    self.editor.panel_open = false;
                } else {
                    self.editor.panel_open = true;
                    self.editor.panel_tab = k;
                }
            }
            x = tr.right() + 14.0;
        }
        let fold = Rect::from_center_size(pos2(head.right() - 18.0, head.center().y), vec2(24.0, 24.0));
        let mut b = ui.new_child(egui::UiBuilder::new().max_rect(fold));
        let glyph = if self.editor.panel_open {
            ph::CARET_DOWN
        } else {
            ph::CARET_UP
        };
        if crate::widgets::tool_button(&mut b, t, glyph, "", false, 24.0).clicked() {
            self.editor.panel_open = !self.editor.panel_open;
        }
        if !self.editor.panel_open {
            return;
        }
        let body = Rect::from_min_max(pos2(r.left(), head.bottom()), r.max);
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(body.shrink2(vec2(14.0, 4.0))));
        child.set_clip_rect(body.intersect(ui.clip_rect()));
        let mut jump = None;
        egui::ScrollArea::both()
            .id_salt(("panel", self.editor.panel_tab))
            .auto_shrink([false; 2])
            .show(&mut child, |ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                let mono = FontId::monospace(CODE_SIZE - 1.0);
                if self.editor.panel_tab == 0 {
                    if readonly {
                        ui.label(
                            egui::RichText::new("Built-in source, read-only. Make a copy to change and run it.")
                                .font(theme::regular(12.5))
                                .color(c.dim),
                        );
                    } else if errors.is_empty() {
                        let msg = if on_chart {
                            "No problems. Running on the chart."
                        } else {
                            concat!(
                                "No problems found. ",
                                shortcut!("Enter"),
                                " saves the script and puts it on the chart."
                            )
                        };
                        ui.label(egui::RichText::new(msg).font(theme::regular(12.5)).color(c.dim));
                    }
                    for e in &errors {
                        let line = error_line(e, &key);
                        let first = e.lines().find(|l| !l.trim().is_empty()).unwrap_or(e);
                        let (rr, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::click());
                        if ui.rect_contains_pointer(rr) {
                            ui.painter().rect_filled(rr, CornerRadius::same(4), t.hover_wash());
                        }
                        let p = ui.painter();
                        p.text(
                            pos2(rr.left() + 10.0, rr.center().y),
                            Align2::CENTER_CENTER,
                            ph::X_CIRCLE,
                            theme::regular(13.0),
                            t.neg,
                        );
                        let g = p.text(
                            pos2(rr.left() + 24.0, rr.center().y),
                            Align2::LEFT_CENTER,
                            first,
                            mono.clone(),
                            c.text,
                        );
                        p.text(
                            pos2(g.right() + 10.0, rr.center().y),
                            Align2::LEFT_CENTER,
                            format!("{}  {}", title, line.map(|l| format!("[Ln {l}]")).unwrap_or_default()),
                            theme::regular(11.5),
                            c.dim,
                        );
                        if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                            jump = line;
                        }
                        // The rest of a long error (a Python traceback).
                        let more: Vec<&str> = e.lines().skip_while(|l| *l != first).skip(1).collect();
                        if !more.is_empty() {
                            ui.add(
                                egui::Label::new(egui::RichText::new(more.join("\n")).font(mono.clone()).color(c.dim))
                                    .extend(),
                            );
                        }
                    }
                } else if !log.is_empty() {
                    ui.add(egui::Label::new(egui::RichText::new(&log).font(mono).color(c.text)).extend());
                }
            });
        if let (Some(n), Some(d)) = (jump, self.editor.docs.get_mut(self.editor.active)) {
            d.jump = Some(n.saturating_sub(1));
        }
    }

    fn status_bar(&mut self, ui: &mut Ui, t: &Theme, r: Rect) {
        let c = code_colors(t);
        let p = ui.painter();
        p.rect_filled(r, CornerRadius::ZERO, c.side);
        p.hline(r.x_range(), r.top() + 0.5, Stroke::new(1.0, c.line));
        let y = r.center().y;
        let font = theme::regular(11.5);
        let mut x = r.left() + 12.0;
        let mut put = |text: &str, col: Color32| {
            let g = p.text(pos2(x, y), Align2::LEFT_CENTER, text, font.clone(), col);
            x = g.right() + 14.0;
        };
        let key = self.editor.doc().map(|d| d.key.clone()).unwrap_or_default();
        let run = self.scripts.iter().find(|x| x.file == key);
        let errs = run.is_some_and(|x| x.error.is_some()) as usize;
        put(
            &format!("{}  {errs}", ph::X_CIRCLE),
            if errs > 0 { t.neg } else { c.dim },
        );
        if self.editor.doc().is_some_and(|d| !d.readonly) {
            match run {
                Some(_) => put(&format!("{}  On the chart", ph::CHART_LINE_UP), t.pos),
                None => put(&format!("{}  Not on the chart", ph::CHART_LINE_UP), c.dim),
            }
        }
        if !self.editor.status.is_empty() {
            put(&self.editor.status, c.text2);
        }
        let Some(d) = self.editor.doc() else { return };
        let (line, col) = self.editor.caret;
        let right = format!(
            "Ln {line}, Col {col}      Spaces: {}      UTF-8      {}",
            indent_unit(d.lang).len(),
            lang_name(d.lang)
        );
        p.text(pos2(r.right() - 12.0, y), Align2::RIGHT_CENTER, right, font, c.dim);
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
}

const TAB_H: f32 = 36.0;
const CRUMB_H: f32 = 26.0;
const STATUS_H: f32 = 24.0;
const PANEL_H: f32 = 118.0;
const MINIMAP_W: f32 = 84.0;
const CODE_SIZE: f32 = 13.5;

/// A folder line in the explorer: caret, NAME, count. Clicking folds it.
fn folder_row(ui: &mut Ui, t: &Theme, title: &str, open: bool, count: usize) -> egui::Response {
    let c = code_colors(t);
    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::click());
    if ui.rect_contains_pointer(r) {
        ui.painter()
            .rect_filled(r, CornerRadius::ZERO, t.hover_wash().gamma_multiply(0.6));
    }
    let p = ui.painter();
    let caret = if open { ph::CARET_DOWN } else { ph::CARET_RIGHT };
    p.text(
        pos2(r.left() + 14.0, r.center().y),
        Align2::CENTER_CENTER,
        caret,
        theme::regular(11.0),
        c.text2,
    );
    p.text(
        pos2(r.left() + 26.0, r.center().y),
        Align2::LEFT_CENTER,
        title,
        theme::semibold(11.0),
        c.text2,
    );
    p.text(
        pos2(r.right() - 12.0, r.center().y),
        Align2::RIGHT_CENTER,
        count.to_string(),
        theme::regular(10.5),
        c.dim,
    );
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A folder in the explorer's tree: caret, folder icon, name, how many
/// scripts are in it. `picked`: new scripts go here.
fn dir_row(ui: &mut Ui, t: &Theme, name: &str, open: bool, depth: usize, count: usize, picked: bool) -> egui::Response {
    let c = code_colors(t);
    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::click());
    let p = ui.painter();
    if picked {
        p.rect_filled(r, CornerRadius::ZERO, t.hover_wash());
    } else if ui.rect_contains_pointer(r) {
        p.rect_filled(r, CornerRadius::ZERO, t.hover_wash().gamma_multiply(0.7));
    }
    let y = r.center().y;
    let x0 = r.left() + 14.0 + depth as f32 * 14.0;
    let caret = if open { ph::CARET_DOWN } else { ph::CARET_RIGHT };
    p.text(
        pos2(x0 - 10.0, y),
        Align2::CENTER_CENTER,
        caret,
        theme::regular(10.0),
        c.dim,
    );
    let icon = if open { ph::FOLDER_OPEN } else { ph::FOLDER_SIMPLE };
    let col = Color32::from_rgb(0xE0, 0xA8, 0x4E);
    p.text(
        pos2(x0 + 6.0, y),
        Align2::CENTER_CENTER,
        icon,
        theme::regular(14.0),
        col,
    );
    let font = theme::regular(13.0);
    let shown = crate::format::elide_to(p, name, &font, r.right() - 34.0 - (x0 + 20.0));
    p.text(pos2(x0 + 20.0, y), Align2::LEFT_CENTER, shown, font, c.text2);
    if count > 0 {
        p.text(
            pos2(r.right() - 12.0, y),
            Align2::RIGHT_CENTER,
            count.to_string(),
            theme::regular(10.5),
            c.dim,
        );
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A folder row that takes a dragged script: it lights up while one hovers
/// and moves it there on drop.
fn drop_target(ui: &Ui, t: &Theme, resp: &egui::Response, dir: &str, action: &mut Option<(u8, String)>) {
    if resp.dnd_hover_payload::<String>().is_some() {
        ui.painter().rect_stroke(
            resp.rect.shrink(1.0),
            CornerRadius::same(4),
            Stroke::new(1.5, t.accent),
            egui::StrokeKind::Inside,
        );
    }
    if let Some(file) = resp.dnd_release_payload::<String>() {
        *action = Some((13, format!("{file}\0{dir}")));
    }
}

/// A folder's right-click menu ("" is the scripts folder itself).
fn folder_menu(ui: &mut Ui, t: &Theme, dir: &str, action: &mut Option<(u8, String)>) {
    use crate::widgets as w;
    ui.set_width(220.0);
    ui.spacing_mut().item_spacing.y = 1.0;
    w::menu_heading(ui, t, "NEW SCRIPT HERE");
    for lang in [Lang::Pine, Lang::Python, Lang::Luau, Lang::Js, Lang::Ts] {
        let (icon, _) = file_icon(lang);
        if w::menu_item(ui, t, Some(icon), lang_name(lang), lang.ext(), false).clicked() {
            *action = Some((14, format!("{dir}\0{}", lang.ext())));
            ui.close();
        }
    }
    w::menu_rule(ui, t);
    if w::menu_item(ui, t, Some(ph::FOLDER_PLUS), "New folder", "", false).clicked() {
        *action = Some((10, dir.to_string()));
        ui.close();
    }
    if w::menu_item(ui, t, Some(ph::FOLDER_OPEN), "Open in your file manager", "", false).clicked() {
        *action = Some((12, dir.to_string()));
        ui.close();
    }
    if !dir.is_empty() {
        if w::menu_item(ui, t, Some(ph::PENCIL_SIMPLE), "Rename", "", false).clicked() {
            *action = Some((4, dir.to_string()));
            ui.close();
        }
        w::menu_rule(ui, t);
        if w::menu_danger(ui, t, Some(ph::TRASH), "Delete folder (if empty)").clicked() {
            *action = Some((11, dir.to_string()));
            ui.close();
        }
    }
}

/// One file in the explorer: its language's icon and name, a dot when
/// it's on the chart, a filled circle when unsaved.
#[allow(clippy::too_many_arguments)]
fn file_row(
    ui: &mut Ui,
    t: &Theme,
    lang: Lang,
    name: &str,
    selected: bool,
    on_chart: bool,
    dirty: bool,
    depth: usize,
) -> egui::Response {
    let c = code_colors(t);
    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::click_and_drag());
    let p = ui.painter();
    if selected {
        p.rect_filled(r, CornerRadius::ZERO, t.accent_soft());
    } else if ui.rect_contains_pointer(r) {
        p.rect_filled(r, CornerRadius::ZERO, t.hover_wash().gamma_multiply(0.7));
    }
    let y = r.center().y;
    let x0 = r.left() + 14.0 + depth as f32 * 14.0;
    let (icon, col) = file_icon(lang);
    p.text(
        pos2(x0 + 6.0, y),
        Align2::CENTER_CENTER,
        icon,
        theme::regular(14.0),
        col,
    );
    let font = theme::regular(13.0);
    let right_pad = if on_chart || dirty { 26.0 } else { 8.0 };
    let shown = crate::format::elide_to(p, name, &font, r.right() - right_pad - (x0 + 20.0));
    p.text(
        pos2(x0 + 20.0, y),
        Align2::LEFT_CENTER,
        shown,
        font,
        if selected { c.text } else { c.text2 },
    );
    if dirty {
        p.circle_filled(pos2(r.right() - 14.0, y), 3.5, c.text2);
    } else if on_chart {
        p.circle_filled(pos2(r.right() - 14.0, y), 3.0, t.pos);
    }
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    if on_chart {
        resp.on_hover_text("On the chart")
    } else {
        resp
    }
}

/// What the outline lists.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Outline {
    Function,
    Input,
    Type,
    Variable,
}

/// The things a script defines at its top level, with their (0-based)
/// lines: functions, types, inputs. A light scan, no parser.
pub fn outline(text: &str, lang: Lang) -> Vec<(usize, String, Outline)> {
    let ident = |s: &str| -> String { s.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect() };
    let mut out = Vec::new();
    for (i, raw) in text.split('\n').enumerate() {
        let top = !raw.starts_with([' ', '\t']);
        let l = raw.trim();
        let after = |p: &str| l.strip_prefix(p).map(|r| ident(r.trim_start()));
        let item = match lang {
            Lang::Pine if top => {
                if let Some(n) = after("type ")
                    .or_else(|| after("export type "))
                    .or_else(|| after("enum "))
                {
                    Some((n, Outline::Type))
                } else if let Some(n) = after("method ").or_else(|| after("export method ")) {
                    Some((n, Outline::Function))
                } else {
                    let l2 = l.strip_prefix("export ").unwrap_or(l);
                    let n = ident(l2);
                    let rest = l2[n.len()..].trim_start();
                    if !n.is_empty() && rest.starts_with('(') && l2.contains("=>") {
                        Some((n, Outline::Function))
                    } else {
                        let l3 = l2.strip_prefix("var ").unwrap_or(l2);
                        let n = ident(l3);
                        let rest = l3[n.len()..].trim_start();
                        (rest.starts_with('=') && !rest.starts_with("==") && rest.contains("input"))
                            .then_some((n, Outline::Input))
                    }
                }
            }
            Lang::Python => {
                if let Some(n) = after("def ") {
                    Some((n, Outline::Function))
                } else if let Some(n) = after("class ") {
                    Some((n, Outline::Type))
                } else if top && l.contains("= input.") {
                    Some((ident(l), Outline::Input))
                } else {
                    None
                }
            }
            Lang::Luau => {
                if let Some(n) = after("local function ").or_else(|| after("function ")) {
                    Some((n, Outline::Function))
                } else if top && l.contains("input.") {
                    after("local ").map(|n| (n, Outline::Input))
                } else {
                    None
                }
            }
            Lang::Js | Lang::Ts => {
                if let Some(n) = after("function ").or_else(|| after("export function ")) {
                    Some((n, Outline::Function))
                } else if top && l.contains("input.") {
                    after("const ").or_else(|| after("let ")).map(|n| (n, Outline::Input))
                } else if top && (l.starts_with("const ") || l.starts_with("let ")) {
                    after("const ")
                        .or_else(|| after("let "))
                        .map(|n| (n, Outline::Variable))
                } else {
                    None
                }
            }
            _ => None,
        };
        if let Some((n, k)) = item
            && !n.is_empty()
        {
            out.push((i, n, k));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folders_in_the_scripts_folder() {
        let mut d = crate::drive::Driver::new("script-folders");
        let root = d.app.scripts_dir();
        std::fs::create_dir_all(root.join("ideas/momentum")).unwrap();
        std::fs::create_dir_all(root.join("empty")).unwrap();
        std::fs::write(root.join("ideas/momentum/rsi.pine"), PINE_TEMPLATE).unwrap();
        std::fs::write(root.join("top.pine"), PINE_TEMPLATE).unwrap();
        assert_eq!(d.app.script_files(), ["ideas/momentum/rsi.pine", "top.pine"]);
        assert_eq!(d.app.script_dirs(), ["empty", "ideas", "ideas/momentum"]);
        // The tree: folders first, open ones show what's in them.
        let mut open = std::collections::HashSet::new();
        assert_eq!(tree(&d.app.script_files(), &d.app.script_dirs(), &open).len(), 3);
        open.insert("ideas".to_string());
        open.insert("ideas/momentum".to_string());
        assert_eq!(tree(&d.app.script_files(), &d.app.script_dirs(), &open).len(), 5);

        // A script in a subfolder opens and runs on the chart.
        d.app.open_in_editor("ideas/momentum/rsi.pine");
        assert_eq!(d.app.editor.doc().unwrap().title, "rsi.pine");
        d.app.add_script("ideas/momentum/rsi.pine");
        d.wait(|a| {
            a.chart
                .studies
                .iter()
                .any(|s| s.meta().id == "pine:ideas/momentum/rsi.pine")
        });

        // On the chart, it stays put; off it, it moves and its tab follows.
        d.app.move_script("ideas/momentum/rsi.pine", "");
        assert!(root.join("ideas/momentum/rsi.pine").exists());
        d.app.scripts.clear();
        d.app.move_script("ideas/momentum/rsi.pine", "");
        assert!(root.join("rsi.pine").exists());
        assert_eq!(d.app.editor.doc().unwrap().key, "rsi.pine");

        // Folders rename; only empty ones delete.
        d.app.rename_path("ideas", "research");
        assert!(root.join("research/momentum").is_dir());
        d.app.move_script("top.pine", "research");
        d.app.delete_folder("research");
        assert!(root.join("research/top.pine").exists(), "a folder with scripts stays");
        d.app.delete_folder("empty");
        assert!(!root.join("empty").exists());
    }

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

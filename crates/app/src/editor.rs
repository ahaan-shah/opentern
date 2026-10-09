//! The Script Editor tab: script files in the workspace, a code editor with
//! highlighting, Save, Add to chart, open in your own editor (the chart
//! reloads when the file changes), and the script's errors and prints.

use crate::app::App;
use crate::icons::ph;
use crate::scripting::Lang;
use crate::theme::{self, Theme};
use egui::text::{LayoutJob, TextFormat};
use egui::{Align, Color32, FontId, Layout, Ui, vec2};
use std::time::SystemTime;

#[derive(Default)]
pub struct Editor {
    pub file: Option<String>,
    pub text: String,
    saved: String,
    mtime: Option<SystemTime>,
    pub status: String,
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

impl App {
    fn open_in_editor(&mut self, file: &str) {
        let path = self.scripts_dir().join(file);
        self.editor.text = std::fs::read_to_string(&path).unwrap_or_default();
        self.editor.saved = self.editor.text.clone();
        self.editor.mtime = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
        self.editor.file = Some(file.to_string());
        self.editor.status.clear();
    }

    fn new_script(&mut self, lang: Lang) {
        let dir = self.scripts_dir();
        let _ = std::fs::create_dir_all(&dir);
        let ext = match lang {
            Lang::Python => "py",
            Lang::Pine => "pine",
        };
        let name = (1..)
            .map(|k| format!("Untitled {k}.{ext}"))
            .find(|n| !dir.join(n).exists())
            .expect("a free name");
        let body = match lang {
            Lang::Python => PYTHON_TEMPLATE,
            Lang::Pine => PINE_TEMPLATE,
        };
        if std::fs::write(dir.join(&name), body).is_ok() {
            self.open_in_editor(&name);
        }
    }

    pub(crate) fn save_script(&mut self) {
        let Some(file) = self.editor.file.clone() else { return };
        let path = self.scripts_dir().join(&file);
        match std::fs::write(&path, &self.editor.text) {
            Ok(()) => {
                self.editor.saved = self.editor.text.clone();
                self.editor.mtime = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
                self.editor.status = "Saved.".into();
                // Pine recompiles on save; Python reloads by itself.
                for i in 0..self.scripts.len() {
                    if self.scripts[i].file == file && self.scripts[i].lang == Lang::Pine {
                        self.compile_pine(i);
                    }
                }
            }
            Err(e) => self.editor.status = format!("Couldn't save: {e}"),
        }
    }

    /// Picks up changes made in another editor (when nothing is unsaved here).
    fn sync_editor_file(&mut self) {
        let Some(file) = self.editor.file.clone() else { return };
        let path = self.scripts_dir().join(&file);
        let now = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
        if now != self.editor.mtime && self.editor.text == self.editor.saved {
            self.open_in_editor(&file);
            self.editor.status = "Reloaded: the file changed on disk.".into();
            for i in 0..self.scripts.len() {
                if self.scripts[i].file == file && self.scripts[i].lang == Lang::Pine {
                    self.compile_pine(i);
                }
            }
        }
    }

    pub(crate) fn editor_tab(&mut self, ui: &mut Ui, t: &Theme) {
        self.sync_editor_file();
        let files = self.script_files();
        if self.editor.file.is_none()
            && let Some(f) = files.first()
        {
            self.open_in_editor(&f.clone());
        }
        let full = ui.available_rect_before_wrap();
        let list_w = 200.0;
        ui.horizontal_top(|ui| {
            // Files.
            ui.allocate_ui_with_layout(vec2(list_w, full.height()), Layout::top_down(Align::Min), |ui| {
                ui.set_min_size(vec2(list_w, full.height()));
                ui.horizontal(|ui| {
                    if ui
                        .button(format!("{} Python", ph::PLUS))
                        .on_hover_text("New Python script")
                        .clicked()
                    {
                        self.new_script(Lang::Python);
                    }
                    if ui
                        .button(format!("{} Pine", ph::PLUS))
                        .on_hover_text("New Pine script")
                        .clicked()
                    {
                        self.new_script(Lang::Pine);
                    }
                });
                ui.add_space(4.0);
                egui::ScrollArea::vertical().id_salt("script-files").show(ui, |ui| {
                    if files.is_empty() {
                        ui.label(egui::RichText::new("No scripts yet.").color(t.text3));
                    }
                    for f in &files {
                        let on = self.editor.file.as_deref() == Some(f.as_str());
                        let icon = match Lang::of(f) {
                            Some(Lang::Python) => ph::FILE_PY,
                            _ => ph::FILE_CODE,
                        };
                        let text =
                            egui::RichText::new(format!("{icon}  {f}")).color(if on { t.accent } else { t.text });
                        if ui.add(egui::Button::new(text).frame(false)).clicked() {
                            self.open_in_editor(f);
                        }
                    }
                });
            });
            ui.separator();
            // Editor.
            ui.vertical(|ui| {
                let Some(file) = self.editor.file.clone() else {
                    ui.label(egui::RichText::new("Create a script to start.").color(t.text3));
                    return;
                };
                let lang = Lang::of(&file).unwrap_or(Lang::Python);
                let dirty = self.editor.text != self.editor.saved;
                ui.horizontal(|ui| {
                    let name = if dirty { format!("{file} •") } else { file.clone() };
                    ui.label(egui::RichText::new(name).font(theme::semibold(13.0)));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui
                            .button(format!("{}  Open folder", ph::FOLDER_OPEN))
                            .on_hover_text("Scripts live here; any editor works")
                            .clicked()
                        {
                            crate::app::open_path(&self.scripts_dir());
                        }
                        if ui
                            .button(format!("{}  Open in editor", ph::ARROW_SQUARE_OUT))
                            .on_hover_text("Edit in your own editor; the chart reloads when you save there")
                            .clicked()
                        {
                            crate::app::open_path(&self.scripts_dir().join(&file));
                        }
                        if ui.button(format!("{}  Add to chart", ph::CHART_LINE_UP)).clicked() {
                            if dirty {
                                self.save_script();
                            }
                            self.add_script(&file);
                        }
                        if ui
                            .add_enabled(dirty, egui::Button::new(format!("{}  Save", ph::FLOPPY_DISK)))
                            .on_hover_text(concat!("Save (", shortcut!("S"), ")"))
                            .clicked()
                        {
                            self.save_script();
                        }
                    });
                });
                // Errors and prints from this script on the chart.
                let run = self.scripts.iter().find(|r| r.file == file);
                let console = run
                    .map(|r| match &r.error {
                        Some(e) => (e.clone(), t.neg),
                        None if !r.log.is_empty() => (r.log.clone(), t.text2),
                        None => ("Running on the chart.".into(), t.text3),
                    })
                    .unwrap_or((self.editor.status.clone(), t.text3));
                let console_h = if console.0.lines().count() > 1 { 90.0 } else { 26.0 };
                let editor_h = (ui.available_height() - console_h - 8.0).max(80.0);
                let font = FontId::monospace(13.0);
                let tt = *t;
                let mut layouter = |ui: &Ui, text: &dyn egui::TextBuffer, wrap: f32| {
                    let mut job = highlight(text.as_str(), lang, &tt, font.clone());
                    job.wrap.max_width = wrap;
                    ui.ctx().fonts_mut(|f| f.layout_job(job))
                };
                egui::ScrollArea::vertical()
                    .id_salt("script-text")
                    .max_height(editor_h)
                    .show(ui, |ui| {
                        let r = ui.add(
                            egui::TextEdit::multiline(&mut self.editor.text)
                                .code_editor()
                                .desired_width(f32::INFINITY)
                                .desired_rows(((editor_h / 17.0) as usize).max(4))
                                .layouter(&mut layouter),
                        );
                        if r.has_focus() && ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::S)) {
                            self.save_script();
                        }
                    });
                egui::ScrollArea::vertical()
                    .id_salt("script-console")
                    .max_height(console_h)
                    .show(ui, |ui| {
                        ui.label(
                            egui::RichText::new(console.0)
                                .font(FontId::monospace(12.0))
                                .color(console.1),
                        );
                    });
            });
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn highlighting_keeps_every_character() {
        let t = crate::theme::Theme::by_name("Opentern Dark");
        for (src, lang) in [(PYTHON_TEMPLATE, Lang::Python), (PINE_TEMPLATE, Lang::Pine)] {
            let job = highlight(src, lang, &t, FontId::monospace(13.0));
            assert_eq!(job.text, src);
            assert!(job.sections.len() > 10);
        }
    }
}

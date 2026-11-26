//! The Indicators dialog: search, favourites, built-ins, the person's own
//! scripts and categories on the left, the list on the right. Click adds to
//! the chart (the dialog stays open, so several can be added); the star
//! marks a favourite.

use super::{Done, frame};
use crate::app::App;
use crate::i18n::tr;
use crate::icons::ph;
use crate::motion;
use crate::theme::{self, Theme};
use egui::{Align2, CornerRadius, Sense, Ui, pos2, vec2};
use opentern_series::builtins;
use opentern_series::study::{Category, Meta};

#[derive(Clone, Copy, PartialEq)]
enum Tab {
    Favorites,
    All,
    /// The person's own indicators and strategies (the scripts folder).
    Mine,
    Cat(Category),
}

/// One row: a built-in (Rust, or a bundled Pine script) or the person's
/// own script.
#[derive(Clone)]
enum Entry {
    Builtin(&'static Meta),
    Script {
        /// Its path in the scripts folder (or `@opentern/…`, bundled).
        file: String,
        name: String,
        strategy: bool,
        /// Bundled scripts are built-ins, with a category.
        bundled: Option<Category>,
    },
}

impl Entry {
    fn name(&self) -> &str {
        match self {
            Entry::Builtin(m) => m.name,
            Entry::Script { name, .. } => name,
        }
    }

    /// Its id for favourites: a built-in's, or the study id a script gets
    /// on the chart (`pine:rsi.pine`).
    fn fav_id(&self) -> String {
        match self {
            Entry::Builtin(m) => m.id.to_string(),
            Entry::Script { file, .. } => match crate::scripting::Lang::of(file) {
                Some(l) => format!("{}:{file}", crate::pine_host::lang_key(l)),
                None => file.clone(),
            },
        }
    }

    fn category(&self) -> Option<Category> {
        match self {
            Entry::Builtin(m) => Some(m.category),
            Entry::Script { bundled, .. } => *bundled,
        }
    }

    fn matches(&self, q: &str) -> bool {
        let n = self.name();
        n.to_lowercase().contains(q)
            || tr(n).to_lowercase().contains(q)
            || match self {
                Entry::Builtin(m) => m.short.to_lowercase().contains(q),
                Entry::Script { file, .. } => file.to_lowercase().contains(q),
            }
    }
}

/// The built-ins: Rust ones and the bundled Pine ones, by name.
fn builtins_list() -> Vec<Entry> {
    let mut v: Vec<Entry> = builtins::all().iter().map(Entry::Builtin).collect();
    v.extend(crate::sources::BUNDLED.iter().map(|b| Entry::Script {
        file: b.file.to_string(),
        name: b.name.to_string(),
        strategy: false,
        bundled: Some(b.category),
    }));
    v.sort_by_key(|e| e.name().to_lowercase());
    v
}

/// The indicators and strategies in the scripts folder (not libraries),
/// named by their declaration's title, else their file.
fn my_scripts(dir: &std::path::Path) -> Vec<Entry> {
    let (files, _) = crate::scripting::scan_scripts(dir);
    let mut v: Vec<Entry> = files
        .into_iter()
        .filter(|f| crate::scripting::Lang::of(f).is_some())
        .filter_map(|file| {
            let src = std::fs::read_to_string(dir.join(&file)).unwrap_or_default();
            let (kind, title) = declaration(&src);
            if kind == Some("library") {
                return None;
            }
            let name = title.unwrap_or_else(|| {
                let base = file.rsplit('/').next().unwrap_or(&file);
                base.rsplit_once('.').map_or(base, |(n, _)| n).to_string()
            });
            Some(Entry::Script {
                file,
                name,
                strategy: kind == Some("strategy"),
                bundled: None,
            })
        })
        .collect();
    v.sort_by_key(|e| e.name().to_lowercase());
    v
}

/// What a script declares itself (`indicator`, `strategy` or `library`)
/// and the title it gives, from the first such call in its text.
fn declaration(src: &str) -> (Option<&'static str>, Option<String>) {
    let code = |l: &&str| {
        let t = l.trim_start();
        !t.starts_with("//") && !t.starts_with('#') && !t.starts_with("--")
    };
    for line in src.lines().filter(code) {
        for kind in ["indicator", "strategy", "library"] {
            let Some(at) = line.find(&format!("{kind}(")) else {
                continue;
            };
            // A whole word (not `my_indicator(`, `strategy.entry(`).
            let before = line[..at].chars().next_back();
            if before.is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '.') {
                continue;
            }
            let rest = &line[at + kind.len() + 1..];
            let title = rest.find(['"', '\'']).and_then(|q| {
                let quote = rest[q..].chars().next()?;
                let body = &rest[q + 1..];
                body.find(quote).map(|e| body[..e].to_string())
            });
            return (Some(kind), title.filter(|t| !t.trim().is_empty()));
        }
    }
    (None, None)
}

pub struct Indicators {
    query: String,
    tab: Tab,
    focused: bool,
    /// The scripts folder's indicators and strategies, read when the
    /// dialog first draws.
    mine: Option<Vec<Entry>>,
}

impl Indicators {
    pub fn new(has_favorites: bool) -> Indicators {
        Indicators {
            query: String::new(),
            tab: if has_favorites { Tab::Favorites } else { Tab::All },
            focused: false,
            mine: None,
        }
    }

    pub fn show(&mut self, ui: &mut Ui, app: &mut App, t: &Theme) -> Done {
        let mut done = Done::Keep;
        let mut add: Option<Entry> = None;
        let mut star: Option<String> = None;
        let mine = self.mine.get_or_insert_with(|| my_scripts(&app.scripts_dir())).clone();
        if frame(ui, t, tr("Indicators"), 720.0, |ui| {
            let r = crate::widgets::search_field(ui, t, &mut self.query, tr("Search"), ui.available_width());
            if !self.focused {
                r.request_focus();
                self.focused = true;
            }
            ui.add_space(8.0);
            // Fits the window: the dialog's title, search and margins take
            // about 200 px.
            let height = (ui.ctx().content_rect().height() - 210.0).clamp(220.0, 460.0);
            ui.horizontal_top(|ui| {
                // Left column: where to look.
                let names = Category::ALL.map(|c| tr(c.name()));
                let left = crate::widgets::widest(ui, theme::medium(13.0), names) + 48.0;
                let left = left.max(170.0);
                ui.allocate_ui_with_layout(vec2(left, height), egui::Layout::top_down(egui::Align::Min), |ui| {
                    ui.set_min_size(vec2(left, height));
                    let mut item = |ui: &mut Ui, tab: Tab, icon: &str, label: &str| {
                        let on = self.tab == tab && self.query.is_empty();
                        let (rect, resp) = ui.allocate_exact_size(vec2(170.0, 30.0), Sense::click());
                        let hov = motion::toggle(ui.ctx(), resp.id, resp.hovered(), motion::MICRO);
                        let bg = if on {
                            t.accent_soft()
                        } else {
                            motion::with_alpha(t.hover_wash(), hov)
                        };
                        ui.painter().rect_filled(rect, CornerRadius::same(7), bg);
                        ui.painter().text(
                            pos2(rect.left() + 20.0, rect.center().y),
                            Align2::CENTER_CENTER,
                            icon,
                            theme::regular(15.0),
                            if on { t.accent } else { t.text2 },
                        );
                        ui.painter().text(
                            pos2(rect.left() + 36.0, rect.center().y),
                            Align2::LEFT_CENTER,
                            label,
                            theme::medium(13.0),
                            if on { t.accent } else { t.text },
                        );
                        if resp.clicked() {
                            self.tab = tab;
                            self.query.clear();
                        }
                    };
                    item(ui, Tab::Favorites, ph::STAR, tr("Favorites"));
                    item(ui, Tab::All, ph::FUNCTION, tr("Built-ins"));
                    item(ui, Tab::Mine, ph::CODE, tr("My scripts"));
                    ui.add_space(6.0);
                    ui.label(
                        egui::RichText::new(tr("Categories"))
                            .font(theme::medium(12.0))
                            .color(t.text3),
                    );
                    ui.add_space(2.0);
                    for c in Category::ALL {
                        item(ui, Tab::Cat(c), category_icon(c), tr(c.name()));
                    }
                });
                let (line, _) = ui.allocate_exact_size(vec2(17.0, height), Sense::hover());
                ui.painter()
                    .vline(line.center().x, line.y_range(), egui::Stroke::new(1.0, t.border));
                // Right: the list.
                let w = ui.available_width();
                ui.allocate_ui_with_layout(vec2(w, height), egui::Layout::top_down(egui::Align::Min), |ui| {
                    let q = self.query.trim().to_lowercase();
                    let list: Vec<Entry> = builtins_list()
                        .into_iter()
                        .chain(mine.iter().cloned())
                        .filter(|e| {
                            if !q.is_empty() {
                                return e.matches(&q);
                            }
                            let own = matches!(e, Entry::Script { bundled: None, .. });
                            match self.tab {
                                Tab::Favorites => app.store.is_favorite("indicator", &e.fav_id()),
                                Tab::All => !own,
                                Tab::Mine => own,
                                Tab::Cat(c) => e.category() == Some(c),
                            }
                        })
                        .collect();
                    egui::ScrollArea::vertical()
                        .id_salt("indicator-list")
                        .max_height(height)
                        .show(ui, |ui| {
                            // Rows stop short of the floating scrollbar.
                            ui.set_width(w - 16.0);
                            if list.is_empty() {
                                if self.tab == Tab::Favorites && q.is_empty() {
                                    crate::widgets::empty_state(ui, t, ph::STAR, tr("No favorites yet"));
                                } else if self.tab == Tab::Mine && q.is_empty() {
                                    crate::widgets::empty_state(ui, t, ph::CODE, tr("No scripts yet"));
                                } else {
                                    crate::widgets::empty_state(ui, t, ph::MAGNIFYING_GLASS, tr("Nothing matches"));
                                }
                            }
                            for e in list {
                                let (rect, resp) =
                                    ui.allocate_exact_size(vec2(ui.available_width(), 32.0), Sense::click());
                                let hov = motion::toggle(ui.ctx(), resp.id, resp.hovered(), motion::MICRO);
                                ui.painter().rect_filled(
                                    rect,
                                    CornerRadius::same(7),
                                    motion::with_alpha(t.hover_wash(), hov),
                                );
                                let fav_id = e.fav_id();
                                let fav = app.store.is_favorite("indicator", &fav_id);
                                let star_rect = egui::Rect::from_center_size(
                                    pos2(rect.left() + 16.0, rect.center().y),
                                    vec2(22.0, 22.0),
                                );
                                let star_resp = ui.interact(star_rect, resp.id.with("star"), Sense::click());
                                let star_col = if fav {
                                    t.warn
                                } else {
                                    let rest = motion::with_alpha(t.text3, 0.3 + hov * 0.7);
                                    motion::lerp_color(rest, t.text2, motion::hover(&star_resp))
                                };
                                let glyph = ph::STAR;
                                ui.painter().text(
                                    star_rect.center(),
                                    Align2::CENTER_CENTER,
                                    glyph,
                                    theme::regular(14.0),
                                    star_col,
                                );
                                ui.painter().text(
                                    pos2(rect.left() + 36.0, rect.center().y),
                                    Align2::LEFT_CENTER,
                                    // The person's own titles stay as they wrote them.
                                    match &e {
                                        Entry::Script {
                                            bundled: None, name, ..
                                        } => name.as_str(),
                                        _ => tr(e.name()),
                                    },
                                    theme::regular(13.5),
                                    t.text,
                                );
                                ui.painter().text(
                                    pos2(rect.right() - 12.0, rect.center().y),
                                    Align2::RIGHT_CENTER,
                                    match &e {
                                        Entry::Script {
                                            bundled: None,
                                            strategy,
                                            ..
                                        } => {
                                            if *strategy {
                                                tr("Strategy")
                                            } else {
                                                tr("Indicator")
                                            }
                                        }
                                        _ => e.category().map_or("", |c| tr(c.name())),
                                    },
                                    theme::regular(12.0),
                                    t.text3,
                                );
                                if star_resp
                                    .on_hover_text(if fav {
                                        tr("Remove from favorites")
                                    } else {
                                        tr("Add to favorites")
                                    })
                                    .clicked()
                                {
                                    star = Some(fav_id);
                                } else if resp.clicked() {
                                    add = Some(e);
                                }
                            }
                        });
                });
            });
        }) {
            done = Done::Close;
        }
        if let Some(id) = star {
            app.toggle_favorite("indicator", &id);
        }
        match add {
            Some(Entry::Builtin(m)) => app.add_study(m),
            Some(Entry::Script { file, .. }) => app.add_script(&file),
            None => {}
        }
        done
    }
}

fn category_icon(c: Category) -> &'static str {
    match c {
        Category::MovingAverages => ph::WAVE_SINE,
        Category::Bands => ph::ARROWS_OUT_LINE_VERTICAL,
        Category::Trend => ph::TREND_UP,
        Category::Oscillators => ph::PULSE,
        Category::Volatility => ph::LIGHTNING,
        Category::Volume => ph::CHART_BAR,
    }
}

#[cfg(test)]
mod tests {
    use super::declaration;

    #[test]
    fn reads_what_a_script_declares() {
        assert_eq!(
            declaration("//@version=6\nstrategy(\"G-FRAMA | Q\", overlay = true)\n"),
            (Some("strategy"), Some("G-FRAMA | Q".into()))
        );
        assert_eq!(
            declaration("// indicator(\"old\")\nindicator(title = 'RSI x')"),
            (Some("indicator"), Some("RSI x".into()))
        );
        assert_eq!(declaration("library(\"Utils\")").0, Some("library"));
        // Calls that only contain the word don't count.
        assert_eq!(
            declaration("x = my_indicator(1)\nstrategy.entry(\"L\", strategy.long)\nindicator(\"Real\")"),
            (Some("indicator"), Some("Real".into()))
        );
        assert_eq!(
            declaration("from opentern import *\nindicator('Py RSI')").1,
            Some("Py RSI".into())
        );
        assert_eq!(declaration("plot(close)"), (None, None));
    }
}

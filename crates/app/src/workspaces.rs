//! Workspaces: the row of numbered pills above the top bar, each its own
//! set of charts (symbol, interval, indicators, drawings shown, scripts, the
//! multi-chart grid). Only the shown one is live; the others are kept as
//! their saved state (`LayoutState`) until they're shown again, so ten or
//! more stay cheap. All of them are saved with the named layout
//! (`LayoutSet`), and come back as they were left.

use crate::app::App;
use crate::i18n::tr;
use crate::icons::ph;
use crate::keys::Action;
use crate::multi::{Grid, LayoutState};
use crate::theme::{self, Theme};
use crate::widgets as w;
use egui::{Align2, CornerRadius, Sense, Stroke, Ui, pos2, vec2};
use serde::{Deserialize, Serialize};

/// The row's height.
pub const BAR_H: f32 = 34.0;

/// How many closed workspaces Ctrl+Shift+T can bring back.
const KEEP_CLOSED: usize = 10;

/// One workspace. The shown one's charts are live in `App` (`saved` is
/// none); the others wait as their saved state.
#[derive(Default)]
pub struct Ws {
    /// A name the person gave it; none follows its main chart's symbol.
    pub name: Option<String>,
    pub saved: Option<LayoutState>,
}

pub struct Workspaces {
    pub list: Vec<Ws>,
    pub current: usize,
    /// Bumped on any change worth saving (switch, add, close, rename, move).
    pub changed: u64,
    /// Closed ones, newest last, with where they were.
    closed: Vec<(usize, Ws)>,
    /// The pill being renamed, and its text so far.
    renaming: Option<(usize, String)>,
    /// The pill being dragged to a new place.
    dragging: Option<usize>,
}

impl Default for Workspaces {
    fn default() -> Self {
        Workspaces {
            list: vec![Ws::default()],
            current: 0,
            changed: 0,
            closed: Vec::new(),
            renaming: None,
            dragging: None,
        }
    }
}

/// A named layout as saved: its workspaces and the one that was shown.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LayoutSet {
    pub version: u32,
    #[serde(default)]
    pub current: usize,
    pub workspaces: Vec<SavedWs>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SavedWs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub layout: LayoutState,
}

impl LayoutSet {
    /// Reads a saved layout, including ones from before workspaces (a
    /// single `LayoutState`, or older still a single chart): one workspace.
    pub fn parse(json: &str) -> Option<LayoutSet> {
        if let Ok(s) = serde_json::from_str::<LayoutSet>(json)
            && !s.workspaces.is_empty()
        {
            return Some(s);
        }
        LayoutState::parse(json).map(LayoutSet::one)
    }

    pub fn one(layout: LayoutState) -> LayoutSet {
        LayoutSet {
            version: 3,
            current: 0,
            workspaces: vec![SavedWs { name: None, layout }],
        }
    }
}

impl App {
    /// The whole named layout: every workspace, the shown one as it is now.
    pub(crate) fn layout_set(&self) -> LayoutSet {
        let workspaces = self
            .ws
            .list
            .iter()
            .enumerate()
            .map(|(i, w)| SavedWs {
                name: w.name.clone(),
                layout: match (&w.saved, i == self.ws.current) {
                    (Some(s), false) => s.clone(),
                    _ => self.layout_state(),
                },
            })
            .collect();
        LayoutSet {
            version: 3,
            current: self.ws.current,
            workspaces,
        }
    }

    /// Puts a saved layout on screen: its workspaces in the row, the one
    /// that was shown live.
    pub(crate) fn install_set(&mut self, set: LayoutSet) {
        let n = set.workspaces.len().max(1);
        let current = set.current.min(n - 1);
        let mut list: Vec<Ws> = set
            .workspaces
            .into_iter()
            .map(|w| Ws {
                name: w.name,
                saved: Some(w.layout),
            })
            .collect();
        let shown = list.get_mut(current).and_then(|w| w.saved.take());
        self.ws = Workspaces {
            list: if list.is_empty() { vec![Ws::default()] } else { list },
            current,
            changed: self.ws.changed + 1,
            ..Workspaces::default()
        };
        if let Some(l) = shown {
            self.install_layout(l);
        }
    }

    /// Shows workspace `i`: the shown one is kept as its saved state and
    /// closed (nothing of it streams), `i`'s charts open.
    pub fn switch_workspace(&mut self, i: usize) {
        let cur = self.ws.current;
        if i == cur || i >= self.ws.list.len() {
            return;
        }
        let Some(next) = self.ws.list[i].saved.take() else {
            return;
        };
        self.chart_ui.draw.cancel();
        self.study_menu = None;
        self.compare_menu = None;
        self.price_menu = None;
        self.alert_menu = None;
        self.chart_menu = None;
        self.layout.overview = false;
        self.ws.list[cur].saved = Some(self.layout_state());
        self.ws.current = i;
        self.ws.renaming = None;
        self.install_layout(next);
        self.ws.changed += 1;
    }

    /// Ctrl+Tab / Ctrl+Shift+Tab: the next or previous one, round the row.
    pub fn step_workspace(&mut self, by: i32) {
        let n = self.ws.list.len() as i32;
        if n > 1 {
            self.switch_workspace((self.ws.current as i32 + by).rem_euclid(n) as usize);
        }
    }

    /// Ctrl+T: a new workspace after this one, on the active chart's symbol
    /// and interval (with its indicators), one chart.
    pub fn new_workspace(&mut self) {
        let layout = LayoutState {
            version: 2,
            grid: Grid::One,
            sync: Default::default(),
            active: 0,
            cells: None,
            charts: vec![self.chart.state()],
        };
        self.insert_workspace(
            self.ws.current + 1,
            Ws {
                name: None,
                saved: Some(layout),
            },
        );
    }

    /// A copy of workspace `i` (all its charts), right after it.
    pub fn duplicate_workspace(&mut self, i: usize) {
        let layout = match self.ws.list.get(i) {
            Some(w) if i != self.ws.current => w.saved.clone(),
            Some(_) => Some(self.layout_state()),
            None => None,
        };
        let name = self.ws.list.get(i).and_then(|w| w.name.clone());
        if let Some(layout) = layout {
            self.insert_workspace(
                i + 1,
                Ws {
                    name,
                    saved: Some(layout),
                },
            );
        }
    }

    fn insert_workspace(&mut self, at: usize, w: Ws) {
        let at = at.min(self.ws.list.len());
        self.ws.list.insert(at, w);
        if at <= self.ws.current {
            self.ws.current += 1;
        }
        self.ws.changed += 1;
        self.switch_workspace(at);
    }

    /// Ctrl+W: closes workspace `i` (not the last one). Closing the shown
    /// one shows its neighbour, the next one first.
    pub fn close_workspace(&mut self, i: usize) {
        let n = self.ws.list.len();
        if n <= 1 || i >= n {
            return;
        }
        if i == self.ws.current {
            self.switch_workspace(if i + 1 < n { i + 1 } else { i - 1 });
        }
        let w = self.ws.list.remove(i);
        if self.ws.current > i {
            self.ws.current -= 1;
        }
        self.ws.closed.push((i, w));
        if self.ws.closed.len() > KEEP_CLOSED {
            self.ws.closed.remove(0);
        }
        self.ws.renaming = None;
        self.ws.changed += 1;
    }

    /// Closes every workspace but `i` (and shows it).
    pub fn close_other_workspaces(&mut self, i: usize) {
        self.switch_workspace(i);
        for k in (0..self.ws.list.len()).rev() {
            if k != self.ws.current {
                self.close_workspace(k);
            }
        }
    }

    /// Ctrl+Shift+T: the last one closed, back where it was.
    pub fn reopen_workspace(&mut self) {
        if let Some((at, w)) = self.ws.closed.pop() {
            self.insert_workspace(at, w);
        }
    }

    /// Names workspace `i`; an empty name goes back to its symbol.
    pub fn rename_workspace(&mut self, i: usize, name: &str) {
        if let Some(w) = self.ws.list.get_mut(i) {
            let name = name.trim();
            w.name = (!name.is_empty()).then(|| name.to_string());
            self.ws.changed += 1;
        }
    }

    /// Moves workspace `from` to place `to` (dragging a pill).
    pub fn move_workspace(&mut self, from: usize, to: usize) {
        let n = self.ws.list.len();
        if from >= n || to >= n || from == to {
            return;
        }
        let w = self.ws.list.remove(from);
        self.ws.list.insert(to, w);
        let c = self.ws.current;
        self.ws.current = if c == from {
            to
        } else if from < c && to >= c {
            c - 1
        } else if from > c && to <= c {
            c + 1
        } else {
            c
        };
        self.ws.changed += 1;
    }

    /// Workspace `i`'s main chart (the first of its grid, the big one where
    /// there is one): its symbol's ticker and interval.
    fn ws_main(&self, i: usize) -> Option<(String, String)> {
        if i == self.ws.current {
            let c = if self.active == 0 {
                &self.chart
            } else {
                &self.slots.first()?.as_ref()?.chart
            };
            return Some((c.symbol.ticker().to_string(), c.tf.label()));
        }
        let c = self.ws.list.get(i)?.saved.as_ref()?.charts.first()?;
        Some((c.symbol.ticker().to_string(), c.tf.label()))
    }

    /// What its pill says: the name given, else its main chart's symbol.
    pub fn ws_title(&self, i: usize) -> String {
        if let Some(n) = self.ws.list.get(i).and_then(|w| w.name.clone()) {
            return n;
        }
        self.ws_main(i)
            .map(|m| m.0)
            .unwrap_or_else(|| tr("Workspace").to_string())
    }

    /// The row above the top bar.
    pub(crate) fn workspace_bar(&mut self, ui: &mut Ui, t: &Theme) {
        let n = self.ws.list.len();
        let mut switch = None;
        let mut close = None;
        let mut close_others = None;
        let mut duplicate = None;
        let mut rename = None;
        let mut drop = None;
        let pointer = ui.ctx().pointer_interact_pos();
        let mut centres = Vec::with_capacity(n);
        ui.horizontal_centered(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            for i in 0..n {
                let title = self.ws_title(i);
                let on = i == self.ws.current;
                // Renaming: a field in the pill's place.
                if let Some((k, text)) = self.ws.renaming.as_mut()
                    && *k == i
                {
                    let r = ui.add(
                        egui::TextEdit::singleline(text)
                            .font(theme::medium(12.5))
                            .margin(vec2(8.0, 3.0))
                            .desired_width(140.0),
                    );
                    if !r.has_focus() && !r.lost_focus() {
                        r.request_focus();
                    }
                    let esc = ui.input(|i| i.key_pressed(egui::Key::Escape));
                    if esc {
                        self.ws.renaming = None;
                    } else if r.lost_focus() {
                        rename = Some((i, text.clone()));
                    }
                    centres.push(r.rect.center().x);
                    continue;
                }
                let font = if on { theme::medium(12.5) } else { theme::regular(12.5) };
                let text_w = ui
                    .painter()
                    .layout_no_wrap(title.clone(), font.clone(), t.text)
                    .size()
                    .x;
                let (r, resp) = ui.allocate_exact_size(vec2(text_w + 35.0, 24.0), Sense::click_and_drag());
                centres.push(r.center().x);
                let hover = crate::motion::hover(&resp);
                let p = ui.painter();
                // The shown one stands out; the others stay quiet.
                if on {
                    p.rect_filled(r, CornerRadius::same(7), t.accent_soft());
                } else {
                    p.rect_filled(
                        r,
                        CornerRadius::same(7),
                        crate::motion::with_alpha(t.hover_wash(), hover),
                    );
                }
                let k = egui::Rect::from_min_size(pos2(r.left() + 4.0, r.center().y - 8.5), vec2(17.0, 17.0));
                let (kbg, kfg) = if on {
                    (t.accent, t.on_accent)
                } else {
                    (t.hover_wash(), t.text3)
                };
                p.rect_filled(k, CornerRadius::same(5), kbg);
                p.text(
                    k.center(),
                    Align2::CENTER_CENTER,
                    (i + 1).to_string(),
                    theme::semibold(10.5),
                    kfg,
                );
                let fg = if on {
                    t.accent
                } else {
                    crate::motion::lerp_color(t.text3, t.text2, hover)
                };
                p.text(
                    pos2(k.right() + 7.0, r.center().y),
                    Align2::LEFT_CENTER,
                    &title,
                    font,
                    fg,
                );
                // Dragging it shows where it would go.
                if resp.dragged() {
                    self.ws.dragging = Some(i);
                }
                if resp.drag_stopped()
                    && let Some(from) = self.ws.dragging.take()
                    && let Some(x) = pointer
                {
                    drop = Some((from, x.x));
                }
                let tip = {
                    let main = self.ws_main(i).map(|(s, tf)| format!("{s} · {tf}")).unwrap_or_default();
                    let key = if i < 9 {
                        self.keys.label(Action::Workspace(i as u8 + 1))
                    } else {
                        String::new()
                    };
                    if key.is_empty() { main } else { format!("{main}  {key}") }
                };
                let resp = if self.ws.dragging.is_none() {
                    resp.on_hover_text(tip)
                } else {
                    resp
                };
                if resp.clicked() {
                    switch = Some(i);
                }
                if resp.double_clicked() {
                    self.ws.renaming = Some((i, title.clone()));
                }
                if resp.middle_clicked() && n > 1 {
                    close = Some(i);
                }
                egui::Popup::context_menu(&resp).show(|ui| {
                    ui.set_width(200.0);
                    ui.spacing_mut().item_spacing.y = 1.0;
                    if w::menu_item(ui, t, Some(ph::PENCIL_SIMPLE), tr("Rename"), "", false).clicked() {
                        self.ws.renaming = Some((i, title.clone()));
                        ui.close();
                    }
                    if w::menu_item(ui, t, Some(ph::COPY), tr("Duplicate"), "", false).clicked() {
                        duplicate = Some(i);
                        ui.close();
                    }
                    w::menu_rule(ui, t);
                    let key = self.keys.label(Action::CloseWorkspace);
                    let hint = if on { key.as_str() } else { "" };
                    if w::menu_row(ui, t, Some(ph::X), tr("Close"), hint, false, n > 1).clicked() {
                        close = Some(i);
                        ui.close();
                    }
                    if w::menu_row(ui, t, Some(ph::X_SQUARE), tr("Close others"), "", false, n > 1).clicked() {
                        close_others = Some(i);
                        ui.close();
                    }
                });
            }
            let tip = self.keys.tip(tr("New workspace"), Action::NewWorkspace);
            if w::tool_button(ui, t, ph::PLUS, &tip, false, 24.0).clicked() {
                self.new_workspace();
            }
        });
        // Where a dragged pill would land: a short accent bar in the gap.
        if let (Some(from), Some(p)) = (self.ws.dragging, pointer) {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
            let to = drop_index(&centres, from, p.x);
            let x = gap_x(&centres, to, from);
            let y = ui.max_rect().center().y;
            ui.painter().vline(x, y - 9.0..=y + 9.0, Stroke::new(2.0, t.accent));
        }
        if let Some((from, x)) = drop {
            self.move_workspace(from, drop_index(&centres, from, x));
        } else if let Some(i) = switch {
            self.switch_workspace(i);
        }
        if let Some((i, name)) = rename {
            self.ws.renaming = None;
            self.rename_workspace(i, &name);
        }
        if let Some(i) = duplicate {
            self.duplicate_workspace(i);
        }
        if let Some(i) = close {
            self.close_workspace(i);
        }
        if let Some(i) = close_others {
            self.close_other_workspaces(i);
        }
    }
}

/// The place a pill dragged from `from` lands with the pointer at `x`:
/// how many of the other pills' middles are left of it.
fn drop_index(centres: &[f32], from: usize, x: f32) -> usize {
    centres
        .iter()
        .enumerate()
        .filter(|(i, c)| *i != from && **c < x)
        .count()
}

/// Where to draw the drop mark for place `to`: between the pills that
/// would be either side of it.
fn gap_x(centres: &[f32], to: usize, from: usize) -> f32 {
    let others: Vec<f32> = centres
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != from)
        .map(|(_, c)| *c)
        .collect();
    match (to.checked_sub(1).and_then(|k| others.get(k)), others.get(to)) {
        (Some(a), Some(b)) => (a + b) / 2.0,
        (Some(a), None) => a + 40.0,
        (None, Some(b)) => b - 40.0,
        (None, None) => 0.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_layouts_load_as_one_workspace() {
        let one = LayoutSet::one(
            LayoutState::parse(r#"{"version":2,"charts":[]}"#).unwrap_or_else(|| LayoutState {
                version: 2,
                grid: Grid::One,
                sync: Default::default(),
                active: 0,
                cells: None,
                charts: Vec::new(),
            }),
        );
        let j = serde_json::to_string(&one).unwrap();
        let back = LayoutSet::parse(&j).unwrap();
        assert_eq!(back.workspaces.len(), 1);
        assert_eq!(back.current, 0);
    }

    #[test]
    fn drops_land_between_the_pills() {
        let c = [10.0, 60.0, 110.0, 160.0];
        assert_eq!(drop_index(&c, 0, 130.0), 2, "past two others");
        assert_eq!(drop_index(&c, 3, 0.0), 0);
        assert_eq!(drop_index(&c, 1, 200.0), 3, "to the end");
    }
}

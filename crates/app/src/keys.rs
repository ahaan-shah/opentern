//! Keyboard shortcuts that can be changed (Settings → Keyboard shortcuts):
//! the actions, their default chords (TradingView's, plus Opentern's own)
//! and the user's changes. The changes are kept in the workspace
//! (`shortcuts` in settings, `{"<action>": "Alt T" | ""}`), read once at
//! launch; frames only look them up.
//!
//! Keys that belong to typing or to a place stay as they are and aren't
//! listed here: typing to search, a number for the interval, "/", the
//! arrows, Delete, Esc, Shift-drag to measure, the editor's and the agent
//! panel's keys.

use crate::chart::draw::Tool;
use crate::i18n::nt;
use egui::{Key, Modifiers};
use opentern_core::Store;
use std::collections::HashMap;

/// Where the changes are kept.
const STORE_KEY: &str = "shortcuts";

/// Something a shortcut does.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Action {
    Alert,
    ResetView,
    Snapshot,
    Watch,
    Overview,
    FullScreen,
    Undo,
    Redo,
    HideDrawings,
    LogScale,
    PercentScale,
    AutoScale,
    /// Workspaces (the row above the top bar), as a browser's tabs.
    NextWorkspace,
    PrevWorkspace,
    NewWorkspace,
    CloseWorkspace,
    ReopenWorkspace,
    /// Workspace 1 to 9.
    Workspace(u8),
    /// A multi-chart layout: the chart left, right, above or below the
    /// active one, and the active one alone (and back).
    ChartLeft,
    ChartRight,
    ChartUp,
    ChartDown,
    MaximizeChart,
    /// Picks a drawing tool.
    Tool(Tool),
}

impl Action {
    /// The general ones, in the sheet's order.
    pub const GENERAL: [Action; 9] = [
        Action::Alert,
        Action::ResetView,
        Action::Snapshot,
        Action::Watch,
        Action::Overview,
        Action::FullScreen,
        Action::Undo,
        Action::Redo,
        Action::HideDrawings,
    ];
    pub const SCALE: [Action; 3] = [Action::LogScale, Action::PercentScale, Action::AutoScale];
    pub const WORKSPACES: [Action; 14] = [
        Action::NextWorkspace,
        Action::PrevWorkspace,
        Action::NewWorkspace,
        Action::CloseWorkspace,
        Action::ReopenWorkspace,
        Action::Workspace(1),
        Action::Workspace(2),
        Action::Workspace(3),
        Action::Workspace(4),
        Action::Workspace(5),
        Action::Workspace(6),
        Action::Workspace(7),
        Action::Workspace(8),
        Action::Workspace(9),
    ];
    pub const CHARTS: [Action; 5] = [
        Action::ChartLeft,
        Action::ChartRight,
        Action::ChartUp,
        Action::ChartDown,
        Action::MaximizeChart,
    ];

    /// Every action: the general ones, the price scale's and the drawing
    /// tools (all but the cursor, which is Esc).
    pub fn all() -> impl Iterator<Item = Action> {
        Action::GENERAL
            .into_iter()
            .chain(Action::SCALE)
            .chain(Action::WORKSPACES)
            .chain(Action::CHARTS)
            .chain(Tool::ALL.into_iter().filter(|t| *t != Tool::Cross).map(Action::Tool))
    }

    /// What it's called on the sheet (an English key; `tr` it).
    pub fn name(self) -> &'static str {
        match self {
            Action::Alert => nt!("Create an alert"),
            Action::ResetView => nt!("Reset the chart view"),
            Action::Snapshot => nt!("Save a snapshot"),
            Action::Watch => nt!("Add to watchlist"),
            Action::Overview => nt!("Market overview"),
            Action::FullScreen => nt!("Full screen"),
            Action::Undo => nt!("Undo"),
            Action::Redo => nt!("Redo"),
            Action::HideDrawings => nt!("Hide all drawings"),
            Action::LogScale => nt!("Log"),
            Action::PercentScale => nt!("Percent"),
            Action::AutoScale => nt!("Auto"),
            Action::NextWorkspace => nt!("Next workspace"),
            Action::PrevWorkspace => nt!("Previous workspace"),
            Action::NewWorkspace => nt!("New workspace"),
            Action::CloseWorkspace => nt!("Close workspace"),
            Action::ReopenWorkspace => nt!("Reopen closed workspace"),
            Action::Workspace(n) => match n {
                1 => nt!("Workspace 1"),
                2 => nt!("Workspace 2"),
                3 => nt!("Workspace 3"),
                4 => nt!("Workspace 4"),
                5 => nt!("Workspace 5"),
                6 => nt!("Workspace 6"),
                7 => nt!("Workspace 7"),
                8 => nt!("Workspace 8"),
                _ => nt!("Workspace 9"),
            },
            Action::ChartLeft => nt!("Chart to the left"),
            Action::ChartRight => nt!("Chart to the right"),
            Action::ChartUp => nt!("Chart above"),
            Action::ChartDown => nt!("Chart below"),
            Action::MaximizeChart => nt!("Maximize chart, or show all"),
            Action::Tool(t) => t.name(),
        }
    }

    /// Its name in the store.
    fn id(self) -> String {
        match self {
            Action::Tool(t) => format!("tool:{}", t.key()),
            other => format!("{other:?}"),
        }
    }

    fn from_id(id: &str) -> Option<Action> {
        if let Some(k) = id.strip_prefix("tool:") {
            return Tool::from_key(k).map(Action::Tool);
        }
        Action::all().find(|a| a.id() == id)
    }

    fn default_chord(self) -> Option<Chord> {
        let s = match self {
            Action::Alert => "Alt A",
            Action::ResetView => "Alt R",
            Action::Snapshot => "Alt S",
            Action::Watch => "Alt W",
            Action::Overview => "Alt M",
            Action::FullScreen => "Shift F",
            Action::Undo => "Ctrl Z",
            Action::Redo => "Ctrl Y",
            Action::HideDrawings => "Ctrl Alt H",
            Action::LogScale => "Alt L",
            Action::PercentScale => "Alt P",
            Action::AutoScale => "Alt U",
            Action::NextWorkspace => "Ctrl Tab",
            Action::PrevWorkspace => "Ctrl Shift Tab",
            Action::NewWorkspace => "Ctrl T",
            Action::CloseWorkspace => "Ctrl W",
            Action::ReopenWorkspace => "Ctrl Shift T",
            Action::Workspace(n) => return Chord::parse(&format!("Alt {n}")),
            Action::ChartLeft => "Alt ArrowLeft",
            Action::ChartRight => "Alt ArrowRight",
            Action::ChartUp => "Alt ArrowUp",
            Action::ChartDown => "Alt ArrowDown",
            Action::MaximizeChart => "Alt Enter",
            Action::Tool(t) => t.shortcut(),
        };
        Chord::parse(s)
    }
}

/// A key with modifiers. `command` is Ctrl, or Cmd on macOS.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Chord {
    pub key: Key,
    pub command: bool,
    pub alt: bool,
    pub shift: bool,
}

impl Chord {
    pub fn new(key: Key, m: Modifiers) -> Chord {
        Chord {
            key,
            command: m.command,
            alt: m.alt,
            shift: m.shift,
        }
    }

    /// From "Ctrl Alt Shift K" (the stored and default form; "Cmd" is
    /// Ctrl too).
    pub fn parse(s: &str) -> Option<Chord> {
        let mut c = Chord {
            key: Key::A,
            command: false,
            alt: false,
            shift: false,
        };
        let mut key = None;
        for word in s.split_whitespace() {
            match word {
                "Ctrl" | "Cmd" => c.command = true,
                "Alt" => c.alt = true,
                "Shift" => c.shift = true,
                k => key = Some(Key::from_name(k)?),
            }
        }
        c.key = key?;
        Some(c)
    }

    /// The stored form: "Ctrl Alt Shift K".
    fn text(self) -> String {
        self.words("Ctrl", self.key.name())
    }

    /// As shown: "Ctrl Alt K" ("Cmd" on macOS), keys by their symbol
    /// where they have one.
    pub fn label(self) -> String {
        let cmd = if cfg!(target_os = "macos") { "Cmd" } else { "Ctrl" };
        self.words(cmd, self.key.symbol_or_name())
    }

    fn words(self, cmd: &str, key: &str) -> String {
        let mut out = Vec::new();
        if self.command {
            out.push(cmd);
        }
        if self.alt {
            out.push("Alt");
        }
        if self.shift {
            out.push("Shift");
        }
        out.push(key);
        out.join(" ")
    }

    /// Whether it can be given to an action: it needs Ctrl (Cmd) or Alt,
    /// as plain keys and Shift+letter are typing (F1 to F12 are fine
    /// alone). Esc, Delete and Backspace keep their job; Enter, Tab and
    /// the arrows only with Ctrl or Alt (alone they type, step and move).
    pub fn usable(self) -> bool {
        use Key::*;
        let fixed = matches!(self.key, Escape | Delete | Backspace);
        let function = matches!(self.key, F1 | F2 | F3 | F4 | F5 | F6 | F7 | F8 | F9 | F10 | F11 | F12);
        !fixed && (self.command || self.alt || function)
    }
}

/// The shortcuts in use: the defaults with the user's changes.
#[derive(Clone, Debug, Default)]
pub struct Keymap {
    /// Changed ones (`None` = taken off).
    changed: HashMap<Action, Option<Chord>>,
}

impl Keymap {
    pub fn load(store: &Store) -> Keymap {
        let mut changed = HashMap::new();
        if let Some(j) = store.value(STORE_KEY)
            && let Ok(map) = serde_json::from_str::<HashMap<String, String>>(&j)
        {
            for (id, chord) in map {
                if let Some(a) = Action::from_id(&id) {
                    changed.insert(a, Chord::parse(&chord));
                }
            }
        }
        Keymap { changed }
    }

    pub fn save(&self, store: &mut Store) {
        let map: HashMap<String, String> = self
            .changed
            .iter()
            .map(|(a, c)| (a.id(), c.map(Chord::text).unwrap_or_default()))
            .collect();
        if let Ok(j) = serde_json::to_string(&map)
            && let Err(e) = store.set_value(STORE_KEY, &j)
        {
            crate::diag::crumb(format!("couldn't save shortcuts: {e}"));
        }
    }

    /// The chord an action has now, if any.
    pub fn chord(&self, a: Action) -> Option<Chord> {
        match self.changed.get(&a) {
            Some(c) => *c,
            None => a.default_chord(),
        }
    }

    /// Its chord as shown ("" when it has none).
    pub fn label(&self, a: Action) -> String {
        self.chord(a).map(Chord::label).unwrap_or_default()
    }

    /// `text` with the action's shortcut after it, for a tooltip:
    /// "Log scale (Alt L)".
    pub fn tip(&self, text: &str, a: Action) -> String {
        match self.chord(a) {
            Some(c) => format!("{text} ({})", c.label()),
            None => text.to_string(),
        }
    }

    /// The action a chord does. When two share it, one the user set wins
    /// over a default (they just gave it those keys).
    pub fn action(&self, c: Chord) -> Option<Action> {
        let mut sharing = Action::all().filter(|a| self.chord(*a) == Some(c));
        let first = sharing.next()?;
        Some(
            std::iter::once(first)
                .chain(sharing)
                .find(|a| !self.is_default(*a))
                .unwrap_or(first),
        )
    }

    /// The other actions with the same keys as `a`.
    pub fn clashes(&self, a: Action) -> Vec<Action> {
        match self.chord(a) {
            Some(c) => Action::all().filter(|b| *b != a && self.chord(*b) == Some(c)).collect(),
            None => Vec::new(),
        }
    }

    pub fn is_default(&self, a: Action) -> bool {
        self.chord(a) == a.default_chord()
    }

    /// Gives `a` the chord (or none). Another action with the same keys
    /// keeps them: the sheet marks both (`clashes`).
    pub fn set(&mut self, a: Action, c: Option<Chord>) {
        if c == a.default_chord() {
            self.changed.remove(&a);
        } else {
            self.changed.insert(a, c);
        }
    }

    /// Back to its default.
    pub fn reset(&mut self, a: Action) {
        self.set(a, a.default_chord());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_all_different_and_usable_or_known() {
        let km = Keymap::default();
        let mut seen = HashMap::new();
        for a in Action::all() {
            let c = km.chord(a).unwrap_or_else(|| panic!("{a:?} has no default"));
            assert!(c.usable() || a == Action::FullScreen, "{a:?}: {c:?}");
            if let Some(b) = seen.insert(c, a) {
                panic!("{a:?} and {b:?} share {}", c.label());
            }
        }
        assert_eq!(
            km.action(Chord::parse("Alt T").unwrap()),
            Some(Action::Tool(Tool::TrendLine))
        );
        assert_eq!(
            km.action(Chord::parse("Alt Shift R").unwrap()),
            Some(Action::Tool(Tool::Rectangle))
        );
        assert_eq!(km.action(Chord::parse("Alt R").unwrap()), Some(Action::ResetView));
    }

    #[test]
    fn chords_read_and_write() {
        for s in ["Alt T", "Ctrl Alt H", "Ctrl Shift Z", "Shift F", "Alt Shift F", "F5"] {
            assert_eq!(Chord::parse(s).unwrap().text(), s);
        }
        assert_eq!(Chord::parse("Cmd Z"), Chord::parse("Ctrl Z"));
        assert!(Chord::parse("Alt Nope").is_none());
        assert!(Chord::parse("Alt").is_none());
        assert!(!Chord::parse("Shift F").unwrap().usable(), "Shift+letter is typing");
        assert!(!Chord::parse("Alt Escape").unwrap().usable());
        assert!(Chord::parse("F5").unwrap().usable());
    }

    #[test]
    fn shared_keys_clash_and_the_changed_one_wins() {
        let mut km = Keymap::default();
        let alt_t = Chord::parse("Alt T").unwrap();
        km.set(Action::Snapshot, Some(alt_t));
        // Both keep Alt T, both are marked, and the one just set wins.
        assert_eq!(km.chord(Action::Tool(Tool::TrendLine)), Some(alt_t));
        assert_eq!(km.clashes(Action::Snapshot), [Action::Tool(Tool::TrendLine)]);
        assert_eq!(km.clashes(Action::Tool(Tool::TrendLine)), [Action::Snapshot]);
        assert_eq!(km.action(alt_t), Some(Action::Snapshot));
        assert!(km.clashes(Action::Undo).is_empty());
        // Reset: no clash, Alt T picks the trend line again.
        km.reset(Action::Snapshot);
        assert!(km.clashes(Action::Snapshot).is_empty());
        assert_eq!(km.action(alt_t), Some(Action::Tool(Tool::TrendLine)));
        assert!(km.changed.is_empty(), "all back to the defaults");
    }

    #[test]
    fn changes_are_kept_in_the_workspace() {
        let dir = std::env::temp_dir().join(format!("opentern-keys-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut store = Store::open(&dir).unwrap();
        let mut km = Keymap::default();
        km.set(Action::Snapshot, Chord::parse("Ctrl Alt S"));
        km.set(Action::Undo, None);
        km.save(&mut store);
        let again = Keymap::load(&Store::open(&dir).unwrap());
        assert_eq!(
            again.label(Action::Snapshot),
            Chord::parse("Ctrl Alt S").unwrap().label()
        );
        assert_eq!(again.chord(Action::Undo), None);
        assert_eq!(again.label(Action::Redo), Chord::parse("Ctrl Y").unwrap().label());
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// The sheet and the app, driven like a person would.
#[cfg(test)]
mod in_the_app {
    use super::*;
    use crate::drive::Driver;
    use crate::i18n::tr;
    use crate::settings::Cat;
    use egui::Event;

    fn press(d: &mut Driver, key: Key, modifiers: Modifiers) {
        let ev = |pressed| Event::Key {
            key,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers,
        };
        d.frame(vec![ev(true)]);
        d.frame(vec![ev(false)]);
    }

    fn alt() -> Modifiers {
        Modifiers {
            alt: true,
            ..Modifiers::NONE
        }
    }

    fn ctrl_alt() -> Modifiers {
        Modifiers {
            alt: true,
            ctrl: true,
            command: true,
            ..Modifiers::NONE
        }
    }

    /// Where `glyph` is painted on `name`'s row of the sheet.
    fn on_row(d: &mut Driver, name: &str, glyph: &str) -> Option<egui::Pos2> {
        let row = d.text_rect(tr(name)).unwrap_or_else(|| panic!("{name}'s row"));
        d.labelled_rects()
            .into_iter()
            .find(|(s, r, _)| s == glyph && r.center().x > row.right() && (r.center().y - row.center().y).abs() < 6.0)
            .map(|(_, r, _)| r.center())
    }

    /// Clicks `name`'s pencil.
    fn edit(d: &mut Driver, name: &str) {
        let pencil = on_row(d, name, crate::icons::ph::PENCIL_SIMPLE).expect("a pencil");
        d.click(pencil);
    }

    /// On the sheet, clicks `name`'s pencil and presses `key`.
    fn record(d: &mut Driver, name: &str, key: Key, modifiers: Modifiers) {
        edit(d, name);
        assert!(d.app.settings.as_ref().unwrap().recording.is_some(), "waiting for keys");
        assert!(d.texts().iter().any(|s| s == tr("Press keys")));
        press(d, key, modifiers);
        assert!(d.app.settings.as_ref().unwrap().recording.is_none(), "done");
    }

    #[test]
    fn a_shortcut_changed_on_the_sheet_works_and_is_kept() {
        let mut d = Driver::new("keys-change");
        d.app.open_settings(Cat::Shortcuts);
        d.frame(vec![]);
        // Save a snapshot: Ctrl Alt S instead of Alt S.
        record(&mut d, "Save a snapshot", Key::S, ctrl_alt());
        assert_eq!(d.app.keys.chord(Action::Snapshot), Chord::parse("Ctrl Alt S"));
        // Esc while waiting keeps what it had, and Settings stays open.
        edit(&mut d, "Create an alert");
        press(&mut d, Key::Escape, Modifiers::NONE);
        assert!(d.app.settings.is_some(), "Esc only stopped the waiting");
        assert_eq!(d.app.keys.label(Action::Alert), Chord::parse("Alt A").unwrap().label());
        // Back on the chart: the new keys take the snapshot, the old don't.
        d.app.settings = None;
        d.frame(vec![]);
        press(&mut d, Key::S, alt());
        assert!(d.app.snap.is_none(), "Alt S does nothing now");
        press(&mut d, Key::S, ctrl_alt());
        assert!(d.app.snap.is_some(), "Ctrl Alt S takes it");
        // Kept in the workspace.
        let again = Keymap::load(&Store::open(&d.dir).unwrap());
        assert_eq!(again.chord(Action::Snapshot), Chord::parse("Ctrl Alt S"));
    }

    #[test]
    fn shared_keys_are_marked_and_reset_brings_the_default_back() {
        let mut d = Driver::new("keys-share");
        d.app.open_settings(Cat::Shortcuts);
        d.frame(vec![]);
        assert!(on_row(&mut d, "Save a snapshot", "!").is_none());
        assert!(on_row(&mut d, "Save a snapshot", crate::icons::ph::ARROW_COUNTER_CLOCKWISE).is_none());
        // Alt T is the trend line's too: both keep it, both get a "!".
        record(&mut d, "Save a snapshot", Key::T, alt());
        assert_eq!(d.app.keys.chord(Action::Tool(Tool::TrendLine)), Chord::parse("Alt T"));
        assert!(on_row(&mut d, "Save a snapshot", "!").is_some());
        assert!(on_row(&mut d, Tool::TrendLine.name(), "!").is_some());
        // Delete takes a row's keys off.
        record(&mut d, "Undo", Key::Delete, Modifiers::NONE);
        assert_eq!(d.app.keys.chord(Action::Undo), None);
        // On the chart, Alt T saves a snapshot (the one just set wins).
        d.app.settings = None;
        d.frame(vec![]);
        press(&mut d, Key::T, alt());
        assert!(d.app.snap.is_some());
        assert_eq!(d.app.layout.tool, Tool::Cross);
        // The reset beside a changed row brings its default back.
        d.app.snap = None;
        d.app.open_settings(Cat::Shortcuts);
        d.frame(vec![]);
        let reset = on_row(&mut d, "Save a snapshot", crate::icons::ph::ARROW_COUNTER_CLOCKWISE).expect("a reset");
        d.click(reset);
        assert!(d.app.keys.is_default(Action::Snapshot));
        assert!(on_row(&mut d, Tool::TrendLine.name(), "!").is_none(), "no clash left");
        d.app.settings = None;
        d.frame(vec![]);
        press(&mut d, Key::T, alt());
        assert_eq!(d.app.layout.tool, Tool::TrendLine);
    }

    #[test]
    fn plain_keys_are_not_taken() {
        let mut d = Driver::new("keys-plain");
        d.app.open_settings(Cat::Shortcuts);
        d.frame(vec![]);
        edit(&mut d, "Save a snapshot");
        // A plain letter is typing: still waiting.
        press(&mut d, Key::K, Modifiers::NONE);
        assert!(d.app.settings.as_ref().unwrap().recording.is_some());
        assert_eq!(
            d.app.keys.label(Action::Snapshot),
            Chord::parse("Alt S").unwrap().label()
        );
    }

    #[test]
    fn the_drawing_bar_stays_off_settings() {
        let mut d = Driver::new("drawbar-settings");
        d.pick(Tool::HorizontalLine);
        d.click(d.at(0.4, 0.5));
        d.key(Key::Escape, Modifiers::NONE);
        d.click(d.at(0.6, 0.5));
        assert!(d.app.chart_ui.draw.selected.is_some(), "selected");
        assert!(d.texts().iter().any(|s| s == tr("Horizontal line")), "its bar shows");
        d.app.open_settings(Cat::General);
        d.frame(vec![]);
        assert!(
            !d.texts().iter().any(|s| s == tr("Horizontal line")),
            "not over Settings"
        );
    }

    /// Long names in a tool list don't run under their shortcut.
    #[test]
    fn tool_lists_fit_their_longest_row() {
        let mut d = Driver::new("flyout-fit");
        let fib = d
            .labelled_rects()
            .into_iter()
            .find(|(s, r, _)| s == Tool::FibRetracement.icon() && r.center().x < 80.0)
            .map(|(_, r, _)| r.center())
            .expect("the Fib group's button");
        let ev = |pressed| Event::PointerButton {
            pos: fib,
            button: egui::PointerButton::Secondary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        d.move_to(fib);
        d.frame(vec![ev(true)]);
        d.frame(vec![ev(false)]);
        d.frame(vec![]);
        let texts = d.labelled_rects();
        let find = |s: &str| texts.iter().find(|(x, ..)| x == s).map(|(_, r, _)| *r);
        let name = find(tr(Tool::FibExtension.name())).expect("the long name, whole");
        let keys = find(&Chord::parse("Alt Shift F").unwrap().label()).expect("its shortcut");
        assert!(name.right() + 8.0 < keys.left(), "{name:?} runs into {keys:?}");
    }
}

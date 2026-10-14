//! Named chart layouts: the chart (symbol, interval, indicators, style,
//! panes) saved under a name, autosaved, and switchable from the top bar.

use crate::app::App;
use crate::chart::{Chart, StudyLook};
use crate::icons::ph;
use crate::multi::{Grid, LayoutState};
use crate::theme::Theme;
use egui::Ui;
use opentern_core::Timeframe;

impl App {
    fn chart_json(&self) -> String {
        serde_json::to_string(&self.layout_state()).unwrap_or_default()
    }

    /// Saves the current layout now.
    pub fn save_layout_now(&mut self) {
        let j = self.chart_json();
        if let Err(e) = self.store.save_layout(self.layout_id, &self.layout_name, &j) {
            crate::diag::crumb(format!("couldn't save layout: {e}"));
        }
        let _ = self.store.set_value("layout_id", &self.layout_id.to_string());
    }

    pub fn load_layout(&mut self, id: i64) {
        if id == self.layout_id {
            return;
        }
        self.save_layout_now();
        let Some(info) = self.store.layouts().iter().find(|l| l.id == id).cloned() else {
            return;
        };
        let Some(state) = self
            .store
            .layout_json(id)
            .ok()
            .flatten()
            .and_then(|j| LayoutState::parse(&j))
        else {
            return;
        };
        self.layout_id = id;
        self.layout_name = info.name;
        self.install_layout(state);
        let _ = self.store.set_value("layout_id", &id.to_string());
    }

    /// A fresh layout: Bitcoin hourly with volume.
    pub fn new_layout(&mut self) {
        self.save_layout_now();
        let btc = opentern_core::starter_watchlist().remove(0);
        let mut c = Chart::new(0, btc, Timeframe::parse("1h").expect("valid"));
        if let Some(v) = opentern_series::builtins::by_id("volume") {
            c.add_study(v, &v.default_inputs(), StudyLook::defaults(v), None);
        }
        let n = self.store.layouts().len() + 1;
        self.layout_name = format!("Layout {n}");
        let state = LayoutState {
            version: 2,
            grid: Grid::One,
            sync: Default::default(),
            active: 0,
            charts: vec![c.state()],
        };
        let json = serde_json::to_string(&state).unwrap_or_default();
        if let Ok(id) = self.store.create_layout(&self.layout_name, &json) {
            self.layout_id = id;
        }
        self.install_layout(state);
        let _ = self.store.set_value("layout_id", &self.layout_id.to_string());
    }

    pub fn copy_layout(&mut self) {
        self.save_layout_now();
        let name = format!("{} copy", self.layout_name);
        if let Ok(id) = self.store.create_layout(&name, &self.chart_json()) {
            self.layout_id = id;
            self.layout_name = name;
            let _ = self.store.set_value("layout_id", &id.to_string());
        }
    }

    pub fn rename_layout(&mut self, name: &str) {
        self.layout_name = name.to_string();
        self.save_layout_now();
    }

    /// The layout menu in the top bar.
    pub(crate) fn layout_menu(&mut self, ui: &mut Ui, t: &Theme) {
        use crate::widgets as w;
        ui.set_width(270.0);
        ui.spacing_mut().item_spacing.y = 1.0;
        w::menu_heading(ui, t, "THIS LAYOUT");
        let id = ui.id().with("rename");
        let mut name: String = ui.data(|d| d.get_temp(id)).unwrap_or_else(|| self.layout_name.clone());
        ui.add_space(3.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let can = !name.trim().is_empty() && name.trim() != self.layout_name;
            let r = ui.add(w::field(&mut name).desired_width(ui.available_width() - 100.0));
            let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if (w::button(ui, w::Kind::Secondary, can, None, "Rename").clicked() || enter) && can {
                self.rename_layout(name.trim());
            }
        });
        ui.add_space(4.0);
        ui.data_mut(|d| d.insert_temp(id, name));
        if w::menu_item(ui, t, Some(ph::FLOPPY_DISK), "Save layout", "", false).clicked() {
            self.save_layout_now();
            ui.close();
        }
        if w::menu_item(ui, t, Some(ph::COPY), "Make a copy", "", false).clicked() {
            self.copy_layout();
            ui.close();
        }
        if w::menu_item(ui, t, Some(ph::FILE_PLUS), "New layout", "", false).clicked() {
            self.new_layout();
            ui.close();
        }
        let others: Vec<_> = self.store.layouts().to_vec();
        let mut open = None;
        let mut delete = None;
        if others.len() > 1 {
            w::menu_rule(ui, t);
            w::menu_heading(ui, t, "OPEN LAYOUT");
            for l in others.iter() {
                let on = l.id == self.layout_id;
                let resp = w::menu_item(ui, t, Some(ph::LAYOUT), &l.name, "", on);
                if !on {
                    // A delete button that shows on hover, at the row's end.
                    let bin = egui::Rect::from_center_size(
                        resp.rect.right_center() - egui::vec2(16.0, 0.0),
                        egui::vec2(24.0, 24.0),
                    );
                    let b = ui.interact(bin, resp.id.with("del"), egui::Sense::click());
                    if resp.hovered() || b.hovered() {
                        ui.painter().text(
                            bin.center(),
                            egui::Align2::CENTER_CENTER,
                            ph::TRASH,
                            crate::theme::regular(14.0),
                            if b.hovered() { t.neg } else { t.text3 },
                        );
                    }
                    if b.on_hover_text("Delete layout").clicked() {
                        delete = Some(l.id);
                    } else if resp.clicked() {
                        open = Some(l.id);
                    }
                }
            }
        }
        if let Some(id) = open {
            self.load_layout(id);
            ui.close();
        }
        if let Some(id) = delete {
            let _ = self.store.delete_layout(id);
        }
    }
}

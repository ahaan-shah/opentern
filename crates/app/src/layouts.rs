//! Named chart layouts: the chart (symbol, interval, indicators, style,
//! panes) saved under a name, autosaved, and switchable from the top bar.

use crate::app::App;
use crate::chart::{Chart, ChartState, StudyLook};
use crate::icons::ph;
use crate::theme::Theme;
use egui::Ui;
use opentern_core::Timeframe;

impl App {
    fn chart_json(&self) -> String {
        serde_json::to_string(&self.chart.state()).unwrap_or_default()
    }

    /// Saves the current layout now.
    pub fn save_layout_now(&mut self) {
        let j = self.chart_json();
        if let Err(e) = self.store.save_layout(self.layout_id, &self.layout_name, &j) {
            crate::diag::crumb(format!("couldn't save layout: {e}"));
        }
        let _ = self.store.set_value("layout_id", &self.layout_id.to_string());
    }

    fn switch_chart(&mut self, chart: Chart) {
        self.save_drawings_now();
        self.chart = chart;
        self.chart_ui = Default::default();
        self.reload_drawings();
        self.open_chart_data();
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
            .and_then(|j| serde_json::from_str::<ChartState>(&j).ok())
        else {
            return;
        };
        self.layout_id = id;
        self.layout_name = info.name;
        let chart = Chart::from_state(self.chart.id + 1, state);
        self.switch_chart(chart);
        let _ = self.store.set_value("layout_id", &id.to_string());
    }

    /// A fresh layout: Bitcoin hourly with volume.
    pub fn new_layout(&mut self) {
        self.save_layout_now();
        let btc = opentern_core::starter_watchlist().remove(0);
        let mut c = Chart::new(self.chart.id + 1, btc, Timeframe::parse("1h").expect("valid"));
        if let Some(v) = opentern_series::builtins::by_id("volume") {
            c.add_study(v, &v.default_inputs(), StudyLook::defaults(v), None);
        }
        let n = self.store.layouts().len() + 1;
        self.layout_name = format!("Layout {n}");
        let state = serde_json::to_string(&c.state()).unwrap_or_default();
        if let Ok(id) = self.store.create_layout(&self.layout_name, &state) {
            self.layout_id = id;
        }
        self.switch_chart(c);
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
        ui.set_min_width(260.0);
        if ui.button(format!("{}  Save layout", ph::FLOPPY_DISK)).clicked() {
            self.save_layout_now();
            ui.close();
        }
        let id = ui.id().with("rename");
        let mut name: String = ui.data(|d| d.get_temp(id)).unwrap_or_else(|| self.layout_name.clone());
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut name).desired_width(170.0));
            if ui
                .add_enabled(
                    !name.trim().is_empty() && name.trim() != self.layout_name,
                    egui::Button::new("Rename"),
                )
                .clicked()
            {
                self.rename_layout(name.trim());
            }
        });
        ui.data_mut(|d| d.insert_temp(id, name));
        if ui.button(format!("{}  Make a copy", ph::COPY)).clicked() {
            self.copy_layout();
            ui.close();
        }
        if ui.button(format!("{}  New layout", ph::FILE_PLUS)).clicked() {
            self.new_layout();
            ui.close();
        }
        let others: Vec<_> = self.store.layouts().to_vec();
        if others.len() > 1 {
            ui.separator();
            ui.label(egui::RichText::new("Open layout").color(t.text3));
        }
        let mut open = None;
        let mut delete = None;
        for l in others.iter() {
            ui.horizontal(|ui| {
                let on = l.id == self.layout_id;
                let text = egui::RichText::new(&l.name).color(if on { t.accent } else { t.text });
                if ui.add(egui::Button::new(text).frame(false)).clicked() {
                    open = Some(l.id);
                }
                if !on {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button(ph::TRASH).on_hover_text("Delete layout").clicked() {
                            delete = Some(l.id);
                        }
                    });
                }
            });
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

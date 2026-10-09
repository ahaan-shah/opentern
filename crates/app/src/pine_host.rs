//! Pine scripts on the chart (the engine lands in the next step).

use crate::app::App;

impl App {
    pub(crate) fn compile_pine(&mut self, i: usize) {
        self.scripts[i].error = Some("Pine Script support is being built.".into());
    }

    pub(crate) fn poll_pine(&mut self, _i: usize) {}
}

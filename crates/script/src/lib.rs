//! Script hosts. Python scripts run in their own process (so a broken
//! script, or a library that crashes, can't take the app down), driven by
//! a worker thread per script that coalesces requests and reloads the
//! script when its file changes.

pub mod python;

pub use python::{Event, InputDesc, Meta, Output, PlotOut, Runner};

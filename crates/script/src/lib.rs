//! Script hosts. Python scripts run in their own process (so a broken
//! script, or a library that crashes, can't take the app down), driven by
//! a worker thread per script that coalesces requests and reloads the
//! script when its file changes. Luau and JavaScript/TypeScript run in
//! process on the shared [`host`], sandboxed, with a memory cap and a time
//! budget per bar.

pub mod host;
pub mod inproc;
pub mod js;
pub mod luau;
pub mod python;

pub use host::{ScriptError, SymInfo};
pub use inproc::{Compiled, ScriptStudy};
pub use python::{Event, InputDesc, Meta, Output, PlotOut, Runner};

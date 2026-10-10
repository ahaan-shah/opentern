//! Opentern's series runtime: Pine-exact technical analysis primitives
//! ([`ta`]), the study model every indicator and script shares
//! ([`study`]), and the built-in indicators ([`builtins`]).

pub mod builtins;
pub mod color;
pub mod drawings;
pub mod study;
pub mod ta;

pub use color::Rgba;
pub use study::{Instance, Meta};

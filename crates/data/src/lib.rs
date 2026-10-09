//! Market data for Opentern. The UI talks to a [`Hub`]; providers fetch
//! history and stream live bars; a local cache keeps what's been
//! downloaded; the resampler builds any interval from a shorter one.

pub mod binance;
pub mod cache;
pub mod demo;
pub mod expr;
pub mod hub;
pub mod provider;
pub mod resample;
pub mod yahoo;

pub use hub::{Event, Hub, Request, Status, Waker};

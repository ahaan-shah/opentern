//! The strategy engine: a broker emulator that fills orders the way
//! TradingView's does, and the Strategy Report computed from its trades
//! (see `docs/metrics.md` for every formula).

pub mod alerts;
pub mod broker;
pub mod paper;
pub mod quant;
pub mod report;

pub use broker::{Broker, Commission, Fill, Lot, OcaType, Order, OrderKind, Props, QtyType, Trade, merge_json};
pub use report::Report;

//! The strategy engine: a broker emulator that fills orders the way
//! TradingView's does, and the Strategy Report computed from its trades
//! (see `docs/metrics.md` for every formula).

pub mod broker;
pub mod report;

pub use broker::{Broker, Order, OrderKind, Props, QtyType, Trade};
pub use report::Report;

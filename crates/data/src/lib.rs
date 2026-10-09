//! Market data for Opentern. The UI talks to a [`Hub`]; providers fetch
//! history and stream live bars; a local cache keeps what's been
//! downloaded; the resampler builds any interval from a shorter one.

pub mod alpaca;
pub mod binance;
pub mod builder;
pub mod bybit;
pub mod cache;
pub mod coinbase;
pub mod creds;
pub mod demo;
pub mod expr;
pub mod hub;
pub mod ibkr;
pub mod kraken;
pub mod oanda;
pub mod okx;
pub mod provider;
pub mod resample;
pub mod upstox;
pub mod yahoo;

pub use creds::Credentials;
pub use hub::{Event, Hub, Request, Status, Waker};

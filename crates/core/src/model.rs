//! Market data primitives. Prices are `f64` (the symbol's `tick_size` decides
//! how they're shown and rounded); times are milliseconds since the Unix
//! epoch, UTC.

use serde::{Deserialize, Serialize};

/// One OHLCV bar. `time` is the bar's open time.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Bar {
    pub time: i64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Unit {
    Second,
    Minute,
    Hour,
    Day,
    Week,
    Month,
}

/// A chart interval: `count` × `unit`, so custom ones like 7m, 3h or 2D work
/// the same as the standard ones.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Timeframe {
    pub count: u32,
    pub unit: Unit,
}

impl Timeframe {
    pub const fn new(count: u32, unit: Unit) -> Timeframe {
        Timeframe { count, unit }
    }

    /// The intervals offered in the interval menu before anyone customises it.
    pub const STANDARD: &[Timeframe] = &[
        Timeframe::new(1, Unit::Minute),
        Timeframe::new(3, Unit::Minute),
        Timeframe::new(5, Unit::Minute),
        Timeframe::new(15, Unit::Minute),
        Timeframe::new(30, Unit::Minute),
        Timeframe::new(45, Unit::Minute),
        Timeframe::new(1, Unit::Hour),
        Timeframe::new(2, Unit::Hour),
        Timeframe::new(3, Unit::Hour),
        Timeframe::new(4, Unit::Hour),
        Timeframe::new(1, Unit::Day),
        Timeframe::new(1, Unit::Week),
        Timeframe::new(1, Unit::Month),
    ];

    /// Parses TradingView-style interval text: "15", "15m", "4h", "1D", "D",
    /// "W", "3M", "30s". A bare number means minutes; letters are
    /// case-insensitive except `m` (minutes) vs `M` (months).
    pub fn parse(s: &str) -> Option<Timeframe> {
        let s = s.trim();
        let split = s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
        let (num, suffix) = s.split_at(split);
        let count = if num.is_empty() { 1 } else { num.parse().ok()? };
        if count == 0 {
            return None;
        }
        let unit = match suffix {
            "" | "m" => Unit::Minute,
            "s" | "S" => Unit::Second,
            "h" | "H" => Unit::Hour,
            "d" | "D" => Unit::Day,
            "w" | "W" => Unit::Week,
            "M" => Unit::Month,
            _ => return None,
        };
        Some(Timeframe { count, unit })
    }

    /// Short label as shown on the interval button: "15m", "4h", "1D".
    pub fn label(&self) -> String {
        let u = match self.unit {
            Unit::Second => "s",
            Unit::Minute => "m",
            Unit::Hour => "h",
            Unit::Day => "D",
            Unit::Week => "W",
            Unit::Month => "M",
        };
        format!("{}{u}", self.count)
    }

    pub fn is_intraday(&self) -> bool {
        matches!(self.unit, Unit::Second | Unit::Minute | Unit::Hour)
    }

    /// Length in milliseconds for fixed-length units; `None` for months,
    /// whose length depends on the calendar.
    pub fn millis(&self) -> Option<i64> {
        let unit: i64 = match self.unit {
            Unit::Second => 1_000,
            Unit::Minute => 60_000,
            Unit::Hour => 3_600_000,
            Unit::Day => 86_400_000,
            Unit::Week => 7 * 86_400_000,
            Unit::Month => return None,
        };
        Some(unit * self.count as i64)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AssetKind {
    Crypto,
    Stock,
    Fund,
    Index,
    Forex,
    Futures,
    /// Gold, oil, wheat…: front-month futures, named like TradingView's
    /// continuous contracts (`COMEX:GC1!`).
    Commodity,
}

/// A tradable symbol. `id` is `EXCHANGE:TICKER` (e.g. `BINANCE:BTCUSDT`,
/// `NSE:RELIANCE`, `NASDAQ:AAPL`), as on TradingView.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Symbol {
    pub id: String,
    pub description: String,
    pub kind: AssetKind,
    pub currency: String,
    pub tick_size: f64,
    /// The smallest tradable quantity (the exchange's lot step,
    /// `syminfo.mincontract`); 0 when unknown.
    #[serde(default)]
    pub min_qty: f64,
    /// The exchange's IANA timezone. Sessions, daily bars and the chart's
    /// "Exchange" time all follow it.
    #[serde(default = "utc")]
    pub timezone: String,
}

fn utc() -> String {
    "UTC".into()
}

impl Symbol {
    /// A symbol known only by its id, until its details load.
    pub fn bare(id: &str) -> Symbol {
        let crypto = id.starts_with("BINANCE:");
        Symbol {
            id: id.to_string(),
            description: String::new(),
            kind: if crypto { AssetKind::Crypto } else { AssetKind::Stock },
            currency: String::new(),
            tick_size: 0.01,
            min_qty: 0.0,
            timezone: utc(),
        }
    }

    pub fn exchange(&self) -> &str {
        self.id.split_once(':').map(|(e, _)| e).unwrap_or("")
    }

    pub fn ticker(&self) -> &str {
        self.id.split_once(':').map(|(_, t)| t).unwrap_or(&self.id)
    }

    /// Decimal places implied by the tick size (0.01 → 2).
    pub fn precision(&self) -> usize {
        let mut p = 0;
        let mut t = self.tick_size;
        while p < 10 && (t - t.round()).abs() > 1e-9 {
            t *= 10.0;
            p += 1;
        }
        p
    }
}

/// The watchlist a brand-new workspace starts with: liquid, public symbols
/// across the markets Opentern covers. Nothing personal.
pub fn starter_watchlist() -> Vec<Symbol> {
    let s = |id: &str, description: &str, kind, currency: &str, tick_size| Symbol {
        id: id.into(),
        description: description.into(),
        kind,
        currency: currency.into(),
        tick_size,
        min_qty: 0.0,
        timezone: match id.split(':').next() {
            Some("NSE" | "BSE") => "Asia/Kolkata".into(),
            Some("AMEX" | "NASDAQ" | "NYSE") => "America/New_York".into(),
            _ => utc(),
        },
    };
    vec![
        s("BINANCE:BTCUSDT", "Bitcoin / TetherUS", AssetKind::Crypto, "USDT", 0.01),
        s(
            "BINANCE:ETHUSDT",
            "Ethereum / TetherUS",
            AssetKind::Crypto,
            "USDT",
            0.01,
        ),
        s("BINANCE:SOLUSDT", "Solana / TetherUS", AssetKind::Crypto, "USDT", 0.01),
        s("AMEX:SPY", "SPDR S&P 500 ETF", AssetKind::Fund, "USD", 0.01),
        s("NASDAQ:QQQ", "Invesco QQQ Trust", AssetKind::Fund, "USD", 0.01),
        s("NASDAQ:AAPL", "Apple Inc.", AssetKind::Stock, "USD", 0.01),
        s("NASDAQ:NVDA", "NVIDIA Corporation", AssetKind::Stock, "USD", 0.01),
        s("NSE:NIFTY", "Nifty 50 Index", AssetKind::Index, "INR", 0.05),
        s("NSE:RELIANCE", "Reliance Industries", AssetKind::Stock, "INR", 0.05),
        s("FX:EURUSD", "Euro / U.S. Dollar", AssetKind::Forex, "USD", 0.00001),
        s("COMEX:GC1!", "Gold", AssetKind::Commodity, "USD", 0.1),
        s("NYMEX:CL1!", "Crude Oil WTI", AssetKind::Commodity, "USD", 0.01),
    ]
}

/// Commodities, as TradingView's continuous front-month futures (`GC1!`).
/// Prices come from Yahoo's front-month futures (`GC=F`).
pub fn commodities() -> Vec<Symbol> {
    let s = |id: &str, description: &str, currency: &str, tick_size| Symbol {
        id: id.into(),
        description: description.into(),
        kind: AssetKind::Commodity,
        currency: currency.into(),
        tick_size,
        min_qty: 0.0,
        timezone: "America/New_York".into(),
    };
    vec![
        s("COMEX:GC1!", "Gold", "USD", 0.1),
        s("COMEX:SI1!", "Silver", "USD", 0.005),
        s("NYMEX:PL1!", "Platinum", "USD", 0.1),
        s("NYMEX:PA1!", "Palladium", "USD", 0.5),
        s("COMEX:HG1!", "Copper", "USD", 0.0005),
        s("NYMEX:CL1!", "Crude Oil WTI", "USD", 0.01),
        s("ICEEUR:BRN1!", "Brent Crude Oil", "USD", 0.01),
        s("NYMEX:NG1!", "Natural Gas", "USD", 0.001),
        s("NYMEX:RB1!", "Gasoline RBOB", "USD", 0.0001),
        s("NYMEX:HO1!", "Heating Oil", "USD", 0.0001),
        s("CBOT:ZW1!", "Wheat", "USX", 0.25),
        s("CBOT:ZC1!", "Corn", "USX", 0.25),
        s("CBOT:ZS1!", "Soybeans", "USX", 0.25),
        s("ICEUS:KC1!", "Coffee", "USX", 0.05),
        s("ICEUS:SB1!", "Sugar", "USX", 0.01),
        s("ICEUS:CC1!", "Cocoa", "USD", 1.0),
        s("ICEUS:CT1!", "Cotton", "USX", 0.01),
        s("CME:LE1!", "Live Cattle", "USX", 0.025),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_tradingview_intervals() {
        let p = |s| Timeframe::parse(s).map(|t| t.label());
        assert_eq!(p("15").as_deref(), Some("15m"));
        assert_eq!(p("15m").as_deref(), Some("15m"));
        assert_eq!(p("4h").as_deref(), Some("4h"));
        assert_eq!(p("D").as_deref(), Some("1D"));
        assert_eq!(p("1d").as_deref(), Some("1D"));
        assert_eq!(p("3M").as_deref(), Some("3M"));
        assert_eq!(p("W").as_deref(), Some("1W"));
        assert_eq!(p("30s").as_deref(), Some("30s"));
        assert_eq!(p("0m"), None);
        assert_eq!(p("5x"), None);
    }

    #[test]
    fn interval_lengths() {
        assert_eq!(Timeframe::parse("4h").unwrap().millis(), Some(4 * 3_600_000));
        assert_eq!(Timeframe::parse("1M").unwrap().millis(), None);
    }

    #[test]
    fn symbol_parts_and_precision() {
        let w = starter_watchlist();
        assert_eq!(w[0].exchange(), "BINANCE");
        assert_eq!(w[0].ticker(), "BTCUSDT");
        assert_eq!(w[0].precision(), 2);
        assert_eq!(w.iter().find(|s| s.id == "FX:EURUSD").unwrap().precision(), 5);
        assert_eq!(w.iter().find(|s| s.id == "NSE:NIFTY").unwrap().precision(), 2);
    }
}

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
        let forex = id.starts_with("FX:");
        Symbol {
            id: id.to_string(),
            description: String::new(),
            kind: if crypto {
                AssetKind::Crypto
            } else if forex {
                AssetKind::Forex
            } else {
                AssetKind::Stock
            },
            currency: String::new(),
            tick_size: if forex {
                forex_tick(id.split_once(':').map_or(id, |(_, t)| t))
            } else {
                0.01
            },
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
        // A currency pair saved before its step was known (0.01) still shows
        // pips and their fraction.
        if self.kind == AssetKind::Forex {
            return decimals(self.tick_size).max(decimals(forex_tick(self.ticker())));
        }
        decimals(self.tick_size)
    }

    /// The price step at `px`. Hyperliquid's perpetuals have none fixed:
    /// it follows the price (`hyperliquid_tick`, sized by the lot step);
    /// every other market keeps its `tick_size`.
    pub fn tick_at(&self, px: f64) -> f64 {
        match self.hyperliquid_sz() {
            Some(sz) if px > 0.0 && px.is_finite() => hyperliquid_tick(px, sz),
            _ => self.tick_size,
        }
    }

    /// Decimal places for a price of `px` (`tick_at`).
    pub fn precision_at(&self, px: f64) -> usize {
        decimals(self.tick_at(px))
    }

    /// Sets `tick_size` to the step at `px`; true when it changed.
    pub fn retick(&mut self, px: f64) -> bool {
        let t = self.tick_at(px);
        if (t - self.tick_size).abs() > t * 1e-9 {
            self.tick_size = t;
            true
        } else {
            false
        }
    }

    /// A Hyperliquid perpetual's size decimals, from its lot step
    /// (`10^-szDecimals`); None for other markets or before the lot loads.
    fn hyperliquid_sz(&self) -> Option<i64> {
        (self.exchange() == "HYPERLIQUID" && self.id.ends_with(".P") && self.min_qty > 0.0)
            .then(|| (-self.min_qty.log10()).round() as i64)
    }
}

/// Decimal places in a step (0.01 → 2).
fn decimals(tick: f64) -> usize {
    let mut p = 0;
    let mut t = tick;
    while p < 10 && (t - t.round()).abs() > 1e-9 {
        t *= 10.0;
        p += 1;
    }
    p
}

/// The price step Hyperliquid allows on a perpetual at `px`: five
/// significant figures, at most `6 - szDecimals` decimals, and whole
/// numbers always.
pub fn hyperliquid_tick(px: f64, sz_decimals: i64) -> f64 {
    let most = 10f64.powi(-(6 - sz_decimals.clamp(0, 6)) as i32);
    let sig = if px > 0.0 && px.is_finite() {
        10f64.powi(px.log10().floor() as i32 - 4)
    } else {
        most
    };
    sig.max(most).min(1.0)
}

/// A currency pair's price step, as quoted by brokers (and TradingView):
/// a tenth of a pip, so 5 decimals, or 3 for pairs priced in yen
/// (USDJPY 156.123). Takes the bare pair (`EURUSD`, `GBPJPY=X`).
pub fn forex_tick(pair: &str) -> f64 {
    let pair: String = pair.strip_suffix("=X").unwrap_or(pair).replace(['_', '/'], "");
    if pair.get(3..6) == Some("JPY") { 0.001 } else { 0.00001 }
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

/// Financial futures, as TradingView's continuous front month (`ZQ1!`),
/// served by Yahoo's front month (`ZQ=F`). Hours like the commodities'.
pub const FED_FUNDS_FUTURES: &str = "CBOT:ZQ1!";

pub fn futures() -> Vec<Symbol> {
    vec![Symbol {
        id: FED_FUNDS_FUTURES.into(),
        // Priced as 100 minus the month's average rate; CBOT's tick for the
        // front month is a quarter of a basis point.
        description: "30-Day Fed Funds Futures".into(),
        kind: AssetKind::Futures,
        currency: "USD".into(),
        tick_size: 0.0025,
        min_qty: 0.0,
        timezone: "America/New_York".into(),
    }]
}

/// Indices, rates and volatility, served by Yahoo: (our id, Yahoo's
/// ticker, name, currency, time zone, tick size). Ids follow TradingView's
/// (`TVC:NI225`, `SP:SPX`, `TVC:US10Y`). Trading hours: `calendar::Market::of`.
pub const INDICES: &[(&str, &str, &str, &str, &str, f64)] = &[
    // United States
    ("SP:SPX", "^GSPC", "S&P 500", "USD", "America/New_York", 0.01),
    ("NASDAQ:NDX", "^NDX", "Nasdaq 100", "USD", "America/New_York", 0.01),
    (
        "NASDAQ:IXIC",
        "^IXIC",
        "Nasdaq Composite",
        "USD",
        "America/New_York",
        0.01,
    ),
    (
        "DJ:DJI",
        "^DJI",
        "Dow Jones Industrial Average",
        "USD",
        "America/New_York",
        0.01,
    ),
    ("TVC:RUT", "^RUT", "Russell 2000", "USD", "America/New_York", 0.01),
    // India
    ("NSE:NIFTY", "^NSEI", "Nifty 50", "INR", "Asia/Kolkata", 0.05),
    ("NSE:BANKNIFTY", "^NSEBANK", "Nifty Bank", "INR", "Asia/Kolkata", 0.05),
    ("BSE:SENSEX", "^BSESN", "S&P BSE Sensex", "INR", "Asia/Kolkata", 0.01),
    // The world
    ("TVC:NI225", "^N225", "Nikkei 225", "JPY", "Asia/Tokyo", 0.01),
    ("TVC:UKX", "^FTSE", "FTSE 100", "GBP", "Europe/London", 0.01),
    ("XETR:DAX", "^GDAXI", "DAX", "EUR", "Europe/Berlin", 0.01),
    ("TVC:CAC40", "^FCHI", "CAC 40", "EUR", "Europe/Paris", 0.01),
    ("TVC:SX5E", "^STOXX50E", "Euro Stoxx 50", "EUR", "Europe/Berlin", 0.01),
    ("SIX:SMI", "^SSMI", "Swiss Market Index", "CHF", "Europe/Zurich", 0.01),
    ("TVC:HSI", "^HSI", "Hang Seng", "HKD", "Asia/Hong_Kong", 0.01),
    (
        "SSE:000001",
        "000001.SS",
        "SSE Composite",
        "CNY",
        "Asia/Shanghai",
        0.001,
    ),
    ("TVC:KOSPI", "^KS11", "KOSPI", "KRW", "Asia/Seoul", 0.01),
    ("TWSE:TAIEX", "^TWII", "Taiwan Weighted", "TWD", "Asia/Taipei", 0.01),
    ("TVC:STI", "^STI", "Straits Times", "SGD", "Asia/Singapore", 0.01),
    ("ASX:XJO", "^AXJO", "S&P/ASX 200", "AUD", "Australia/Sydney", 0.1),
    (
        "TSX:TSX",
        "^GSPTSE",
        "S&P/TSX Composite",
        "CAD",
        "America/Toronto",
        0.01,
    ),
    ("BMFBOVESPA:IBOV", "^BVSP", "Ibovespa", "BRL", "America/Sao_Paulo", 1.0),
    // Rates, volatility and the dollar
    (
        "TVC:US03MY",
        "^IRX",
        "US 3-month Treasury bill yield",
        "%",
        "America/New_York",
        0.001,
    ),
    (
        "TVC:US05Y",
        "^FVX",
        "US 5-year Treasury yield",
        "%",
        "America/New_York",
        0.001,
    ),
    (
        "TVC:US10Y",
        "^TNX",
        "US 10-year Treasury yield",
        "%",
        "America/New_York",
        0.001,
    ),
    (
        "TVC:US30Y",
        "^TYX",
        "US 30-year Treasury yield",
        "%",
        "America/New_York",
        0.001,
    ),
    (
        "CBOE:VIX",
        "^VIX",
        "Cboe Volatility Index",
        "USD",
        "America/New_York",
        0.01,
    ),
    (
        "CBOE:VXN",
        "^VXN",
        "Cboe Nasdaq 100 Volatility Index",
        "USD",
        "America/New_York",
        0.01,
    ),
    (
        "TVC:MOVE",
        "^MOVE",
        "ICE BofA MOVE Index (bond volatility)",
        "USD",
        "America/New_York",
        0.01,
    ),
    ("NSE:INDIAVIX", "^INDIAVIX", "India VIX", "INR", "Asia/Kolkata", 0.0025),
    (
        "TVC:DXY",
        "DX-Y.NYB",
        "U.S. Dollar Index",
        "USD",
        "America/New_York",
        0.001,
    ),
];

/// Every index, rate and volatility index above, as symbols, and the Fed
/// funds rate.
pub fn indices() -> Vec<Symbol> {
    INDICES
        .iter()
        .map(|(id, _, name, currency, tz, tick)| Symbol {
            id: (*id).into(),
            description: (*name).into(),
            kind: AssetKind::Index,
            currency: (*currency).into(),
            tick_size: *tick,
            min_qty: 0.0,
            timezone: (*tz).into(),
        })
        .chain(std::iter::once(fed_funds()))
        .collect()
}

/// The effective federal funds rate, from the New York Fed (not Yahoo).
pub const FED_FUNDS: &str = "NYFED:EFFR";

/// The effective federal funds rate as a symbol: one value a business day.
pub fn fed_funds() -> Symbol {
    Symbol {
        id: FED_FUNDS.into(),
        description: "US Effective Federal Funds Rate".into(),
        kind: AssetKind::Index,
        currency: "%".into(),
        tick_size: 0.01,
        min_qty: 0.0,
        timezone: "America/New_York".into(),
    }
}

/// Other words search finds a symbol by (upper case, as typed).
pub fn search_aliases(id: &str) -> &'static [&'static str] {
    match id {
        FED_FUNDS => &[
            "FED FUNDS",
            "FED FUNDS RATE",
            "FEDFUNDS",
            "FED RATE",
            "INTEREST RATE",
            "DFF",
        ],
        FED_FUNDS_FUTURES => &[
            "ZQ",
            "ZQ=F",
            "ZQ1",
            "FED FUNDS FUTURES",
            "FED FUNDS FUTURE",
            "30-DAY FED FUNDS",
            "30 DAY FED FUNDS",
        ],
        _ => &[],
    }
}

/// Whether a symbol is published once a day rather than traded.
pub fn daily_only(id: &str) -> bool {
    id == FED_FUNDS
}

/// Our index id → Yahoo's ticker (`TVC:NI225` → `^N225`).
pub fn index_yahoo(id: &str) -> Option<&'static str> {
    INDICES.iter().find(|i| i.0 == id).map(|i| i.1)
}

/// Yahoo's ticker → our index id (`^N225` → `TVC:NI225`).
pub fn index_from_yahoo(ticker: &str) -> Option<&'static str> {
    INDICES.iter().find(|i| i.1 == ticker).map(|i| i.0)
}

/// Spot commodities from OANDA (needs the user's OANDA key): the price
/// of the metal or barrel now, not a futures contract. Futures trade above
/// spot by the cost of carry, so the two never show the same number.
pub fn spot_commodities() -> Vec<Symbol> {
    let s = |t: &str, description: &str, tick_size| Symbol {
        id: format!("OANDA:{t}"),
        description: description.into(),
        kind: AssetKind::Commodity,
        currency: "USD".into(),
        tick_size,
        min_qty: 0.0,
        timezone: "America/New_York".into(),
    };
    vec![
        s("XAUUSD", "Gold Spot / U.S. Dollar", 0.01),
        s("XAGUSD", "Silver Spot / U.S. Dollar", 0.001),
        s("XPTUSD", "Platinum Spot / U.S. Dollar", 0.01),
        s("XPDUSD", "Palladium Spot / U.S. Dollar", 0.01),
        s("XCUUSD", "Copper Spot / U.S. Dollar", 0.0001),
        s("WTICOUSD", "Crude Oil WTI Spot", 0.001),
        s("BCOUSD", "Brent Crude Oil Spot", 0.001),
        s("NATGASUSD", "Natural Gas Spot", 0.0001),
        s("WHEATUSD", "Wheat Spot", 0.001),
        s("CORNUSD", "Corn Spot", 0.001),
        s("SOYBNUSD", "Soybeans Spot", 0.001),
        s("SUGARUSD", "Sugar Spot", 0.00001),
    ]
}

/// Which commodity a symbol is, futures or spot: "gold", "wti", …
/// (None for anything else). Names its icon.
pub fn commodity_of(s: &Symbol) -> Option<&'static str> {
    if s.kind != AssetKind::Commodity {
        return None;
    }
    let t = s.ticker().to_uppercase();
    let root = t.trim_end_matches('!').trim_end_matches('1').trim_end_matches("USD");
    Some(match root {
        "GC" | "XAU" => "gold",
        "SI" | "XAG" => "silver",
        "PL" | "XPT" => "platinum",
        "PA" | "XPD" => "palladium",
        "HG" | "XCU" => "copper",
        "CL" | "WTICO" => "wti",
        "BRN" | "BZ" | "BCO" => "brent",
        "NG" | "NATGAS" => "natgas",
        "RB" => "gasoline",
        "HO" => "heating-oil",
        "ZW" | "WHEAT" => "wheat",
        "ZC" | "CORN" => "corn",
        "ZS" | "SOYBN" => "soybeans",
        "KC" => "coffee",
        "SB" | "SUGAR" => "sugar",
        "CC" => "cocoa",
        "CT" => "cotton",
        "LE" => "cattle",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hyperliquid_steps_follow_the_live_price() {
        let mut s = Symbol::bare("HYPERLIQUID:BTCUSDC.P");
        s.min_qty = 0.00001; // szDecimals 5
        s.tick_size = 1.0;
        assert_eq!(s.precision_at(59_819.0), 0);
        assert_eq!(s.precision_at(9_500.0), 1);
        assert!(s.retick(9_500.0));
        assert_eq!(s.precision(), 1);
        assert!(!s.retick(9_400.0));
        assert!(s.retick(10_200.0));
        assert_eq!(s.tick_size, 1.0);
        // A coin around 1, capped at 6 - szDecimals decimals.
        let mut c = Symbol::bare("HYPERLIQUID:ARBUSDC.P");
        c.min_qty = 0.1;
        assert_eq!(c.precision_at(0.99), 5);
        assert_eq!(c.precision_at(1.01), 4);
        // Before the lot step loads, and on other markets, the fixed step.
        c.min_qty = 0.0;
        c.tick_size = 0.001;
        assert_eq!(c.tick_at(0.99), 0.001);
        let mut b = Symbol::bare("BINANCE:BTCUSDT");
        b.min_qty = 0.00001;
        assert!(!b.retick(9_500.0));
        assert_eq!(b.tick_at(9_500.0), 0.01);
    }

    #[test]
    fn indices_map_both_ways() {
        assert_eq!(index_yahoo("TVC:NI225"), Some("^N225"));
        assert_eq!(index_from_yahoo("^TNX"), Some("TVC:US10Y"));
        let all = indices();
        let mut ids: Vec<_> = all.iter().map(|s| s.id.clone()).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), all.len(), "ids are unique");
        // Every index's time zone is a real one.
        for s in &all {
            assert!(jiff::tz::TimeZone::get(&s.timezone).is_ok(), "{}", s.id);
        }
    }

    #[test]
    fn futures_and_spot_name_the_same_commodity() {
        let of = |id: &str| {
            let all: Vec<Symbol> = commodities().into_iter().chain(spot_commodities()).collect();
            commodity_of(all.iter().find(|s| s.id == id).unwrap())
        };
        assert_eq!(of("COMEX:GC1!"), Some("gold"));
        assert_eq!(of("OANDA:XAUUSD"), Some("gold"));
        assert_eq!(of("ICEEUR:BRN1!"), Some("brent"));
        assert_eq!(of("OANDA:BCOUSD"), Some("brent"));
        assert_eq!(of("OANDA:NATGASUSD"), Some("natgas"));
        assert!(
            commodities()
                .iter()
                .chain(&spot_commodities())
                .all(|s| commodity_of(s).is_some())
        );
        assert_eq!(commodity_of(&starter_watchlist()[0]), None);
    }

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
        // Pairs found by search or typed: a tenth of a pip, yen pairs 3.
        assert_eq!(Symbol::bare("FX:EURGBP").precision(), 5);
        assert_eq!(Symbol::bare("FX:GBPJPY").precision(), 3);
        assert_eq!(forex_tick("USDJPY=X"), 0.001);
        assert_eq!(forex_tick("EUR_JPY"), 0.001);
        // One saved with a cent step before still shows 5.
        let mut old = Symbol::bare("FX:AUDUSD");
        old.tick_size = 0.01;
        assert_eq!(old.precision(), 5);
        assert_eq!(w.iter().find(|s| s.id == "NSE:NIFTY").unwrap().precision(), 2);
    }
}

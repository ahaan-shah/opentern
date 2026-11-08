//! Number and time formatting for scales, legends and the watchlist.

use opentern_core::{Symbol, Timeframe};
use opentern_data::overview::Market;

/// A price with fixed decimals and no thousands separators (as on
/// TradingView's price scale).
pub fn price(v: f64, decimals: usize) -> String {
    if !v.is_finite() {
        return "—".into();
    }
    format!("{v:.decimals$}")
}

/// A number with thousands separators: 20,068.36.
pub fn grouped(v: f64, decimals: usize) -> String {
    if !v.is_finite() {
        return "—".into();
    }
    let s = format!("{:.decimals$}", v.abs());
    let (int, frac) = s
        .split_once('.')
        .map(|(i, f)| (i, Some(f)))
        .unwrap_or((s.as_str(), None));
    let mut out = String::new();
    for (k, ch) in int.chars().enumerate() {
        if k > 0 && (int.len() - k) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    if let Some(f) = frac {
        out.push('.');
        out.push_str(f);
    }
    if v < 0.0 && s.chars().any(|c| c.is_ascii_digit() && c != '0') {
        out.insert(0, '−');
    }
    out
}

/// How big numbers read for a market: millions, billions and trillions,
/// or lakhs and crores. Set per market in Settings → General.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Numbers {
    Intl,
    Indian,
}

impl Numbers {
    pub fn key(self) -> &'static str {
        match self {
            Numbers::Intl => "intl",
            Numbers::Indian => "indian",
        }
    }

    pub fn from_key(s: &str) -> Option<Numbers> {
        match s {
            "intl" => Some(Numbers::Intl),
            "indian" => Some(Numbers::Indian),
            _ => None,
        }
    }

    /// Indian stocks in crores, the rest in billions.
    pub fn default_for(m: Market) -> Numbers {
        match m {
            Market::India => Numbers::Indian,
            _ => Numbers::Intl,
        }
    }
}

/// The settings key holding each market's style.
pub const NUMBERS_KEY: &str = "number_style";

static STYLES: std::sync::Mutex<[Option<Numbers>; 3]> = std::sync::Mutex::new([None; 3]);

fn slot(m: Market) -> usize {
    match m {
        Market::Us => 0,
        Market::India => 1,
        Market::Crypto => 2,
    }
}

/// The style a market's numbers use.
pub fn numbers(m: Market) -> Numbers {
    STYLES
        .lock()
        .ok()
        .and_then(|s| s[slot(m)])
        .unwrap_or_else(|| Numbers::default_for(m))
}

pub fn set_numbers(m: Market, n: Numbers) {
    if let Ok(mut s) = STYLES.lock() {
        s[slot(m)] = Some(n);
    }
}

/// Loads the styles saved as `{"us":"intl","india":"indian",...}`.
pub fn load_numbers(json: Option<&str>) {
    let v: serde_json::Value = json.and_then(|s| serde_json::from_str(s).ok()).unwrap_or_default();
    for m in Market::ALL {
        let n = v[m.key()]
            .as_str()
            .and_then(Numbers::from_key)
            .unwrap_or_else(|| Numbers::default_for(m));
        set_numbers(m, n);
    }
}

pub fn numbers_json() -> String {
    let mut o = serde_json::Map::new();
    for m in Market::ALL {
        o.insert(m.key().into(), numbers(m).key().into());
    }
    serde_json::Value::Object(o).to_string()
}

/// Which market's style a symbol's numbers follow: NSE/BSE are India,
/// coins are crypto, everything else reads like US stocks.
pub fn market_of(s: &Symbol) -> Market {
    if s.kind == opentern_core::AssetKind::Crypto {
        Market::Crypto
    } else if matches!(s.exchange(), "NSE" | "BSE") || s.id.ends_with(".NS") || s.id.ends_with(".BO") {
        Market::India
    } else {
        Market::Us
    }
}

/// A big number in a market's style: 1.25K, 3.4M, 2.1B, 4.51T; or
/// 1.25K, 3.4L, 2.1Cr, and grouped crores past a thousand crore
/// (9,04,393Cr).
pub fn big(v: f64, m: Market) -> String {
    match numbers(m) {
        Numbers::Intl => compact(v),
        Numbers::Indian => indian(v),
    }
}

/// Volumes and other counts for a symbol, in its market's style.
pub fn compact_for(v: f64, s: &Symbol) -> String {
    big(v, market_of(s))
}

/// Lakhs and crores.
pub fn indian(v: f64) -> String {
    if !v.is_finite() {
        return "—".into();
    }
    let a = v.abs();
    if a >= 1e10 {
        // A thousand crore and up: whole crores, grouped the Indian way.
        let sign = if v < 0.0 { "−" } else { "" };
        return format!("{sign}{}Cr", indian_grouped((a / 1e7).round() as u64));
    }
    let (d, s) = if a >= 1e7 {
        (1e7, "Cr")
    } else if a >= 1e5 {
        (1e5, "L")
    } else if a >= 1e3 {
        (1e3, "K")
    } else {
        return trim(format!("{v:.2}"));
    };
    format!("{}{s}", trim(format!("{:.2}", v / d)))
}

/// 9043930 → "90,43,930".
pub fn indian_grouped(n: u64) -> String {
    let s = n.to_string();
    if s.len() <= 3 {
        return s;
    }
    let (head, last3) = s.split_at(s.len() - 3);
    let mut out = String::new();
    for (k, ch) in head.chars().enumerate() {
        if k > 0 && (head.len() - k) % 2 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    format!("{out},{last3}")
}

/// Volume-style compact numbers: 950, 1.25K, 3.4M, 2.1B, 4.51T.
pub fn compact(v: f64) -> String {
    if !v.is_finite() {
        return "—".into();
    }
    let a = v.abs();
    let (d, s) = if a >= 1e12 {
        (1e12, "T")
    } else if a >= 1e9 {
        (1e9, "B")
    } else if a >= 1e6 {
        (1e6, "M")
    } else if a >= 1e3 {
        (1e3, "K")
    } else {
        return trim(format!("{v:.2}"));
    };
    format!("{}{s}", trim(format!("{:.2}", v / d)))
}

fn trim(s: String) -> String {
    if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        s
    }
}

/// Decimals that make `step` (a scale step) readable, at least `min`.
pub fn decimals_for(step: f64, min: usize) -> usize {
    if !(step.is_finite() && step > 0.0) {
        return min;
    }
    let mut d = 0;
    let mut s = step;
    while d < 10 && (s - s.round()).abs() > 1e-9 * s.max(1.0) {
        s *= 10.0;
        d += 1;
    }
    d.max(min)
}

pub fn signed_pct(v: f64) -> String {
    if !v.is_finite() {
        return "—".into();
    }
    format!("{}{:.2}%", if v > 0.0 { "+" } else { "" }, v)
}

/// The time under the crosshair: "Tue 09 Oct '26 14:35" for intraday,
/// "Tue 09 Oct '26" for daily, "Oct '26" for monthly.
pub fn crosshair_time(t: i64, tf: Timeframe, tz: &jiff::tz::TimeZone) -> String {
    let Ok(ts) = jiff::Timestamp::from_millisecond(t) else {
        return String::new();
    };
    // Daily and longer bars are stamped at UTC midnight of their date.
    let z = if tf.is_intraday() {
        ts.to_zoned(tz.clone())
    } else {
        ts.to_zoned(jiff::tz::TimeZone::UTC)
    };
    match tf.unit {
        opentern_core::Unit::Month => z.strftime("%b '%y").to_string(),
        opentern_core::Unit::Day | opentern_core::Unit::Week => z.strftime("%a %d %b '%y").to_string(),
        opentern_core::Unit::Second => z.strftime("%a %d %b '%y %H:%M:%S").to_string(),
        _ => z.strftime("%a %d %b '%y %H:%M").to_string(),
    }
}

/// "12:04:31" or "3d 04:12" until the forming bar closes.
pub fn countdown(ms: i64) -> String {
    let s = (ms / 1000).max(0);
    let (d, h, m, s) = (s / 86_400, s / 3600 % 24, s / 60 % 60, s % 60);
    if d > 0 {
        format!("{d}d {h:02}:{m:02}")
    } else if h > 0 {
        format!("{h:02}:{m:02}:{s:02}")
    } else {
        format!("{m:02}:{s:02}")
    }
}

/// `text` cut to fit `max_w` points in `font`, ending in "…" when cut.
pub fn elide_to(p: &egui::Painter, text: &str, font: &egui::FontId, max_w: f32) -> String {
    let fits = |s: &str| {
        p.layout_no_wrap(s.to_string(), font.clone(), egui::Color32::WHITE)
            .size()
            .x
            <= max_w
    };
    if fits(text) {
        return text.to_string();
    }
    let chars: Vec<char> = text.chars().collect();
    for n in (1..chars.len()).rev() {
        let cut: String = chars[..n].iter().collect::<String>() + "…";
        if fits(&cut) {
            return cut;
        }
    }
    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lakhs_and_crores() {
        assert_eq!(compact(4.51e12), "4.51T");
        assert_eq!(indian(950.0), "950");
        assert_eq!(indian(250_000.0), "2.5L");
        assert_eq!(indian(31_200_000.0), "3.12Cr");
        assert_eq!(indian(9_000_000_000.0), "900Cr");
        assert_eq!(indian(9.04393e12), "9,04,393Cr");
        assert_eq!(indian_grouped(1_234_567_890), "1,23,45,67,890");
        assert_eq!(indian_grouped(999), "999");
        let sym = |id: &str, kind| Symbol {
            kind,
            ..Symbol::bare(id)
        };
        use opentern_core::AssetKind;
        assert_eq!(market_of(&sym("NSE:TCS", AssetKind::Stock)), Market::India);
        assert_eq!(market_of(&sym("BINANCE:BTCUSDT", AssetKind::Crypto)), Market::Crypto);
        assert_eq!(market_of(&sym("NASDAQ:AAPL", AssetKind::Stock)), Market::Us);
    }

    #[test]
    fn numbers() {
        assert_eq!(price(1234.5, 2), "1234.50");
        assert_eq!(compact(950.0), "950");
        assert_eq!(compact(1250.0), "1.25K");
        assert_eq!(compact(3_400_000.0), "3.4M");
        assert_eq!(compact(-2_100_000_000.0), "-2.1B");
        assert_eq!(decimals_for(0.25, 0), 2);
        assert_eq!(decimals_for(50.0, 0), 0);
        assert_eq!(decimals_for(0.0005, 2), 4);
        assert_eq!(signed_pct(1.234), "+1.23%");
        assert_eq!(countdown(3_725_000), "01:02:05");
        assert_eq!(countdown(65_000), "01:05");
    }
}

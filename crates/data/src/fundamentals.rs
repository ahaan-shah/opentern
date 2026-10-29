//! Company fundamentals for custom indices: market cap and shares (the
//! weights), valuation (P/E, EV/EBITDA), growth and margins. From Yahoo's
//! `quoteSummary` (unofficial, best effort, like its charts), which needs
//! the session cookie and "crumb" every Yahoo page gets; both are fetched
//! once and reused. Demo mode makes numbers up.
//!
//! Ratios are as Yahoo reports them (trailing twelve months); margins and
//! growth are fractions here and percent in the app.

use crate::provider::{Error, Result};
use opentern_core::{AssetKind, Symbol};
use std::sync::Mutex;

/// What's known about one company. NaN where the source has nothing.
#[derive(Clone, Debug, PartialEq)]
pub struct Fundamentals {
    pub market_cap: f64,
    pub shares: f64,
    pub pe: f64,
    pub ev_ebitda: f64,
    pub enterprise_value: f64,
    pub ebitda: f64,
    pub revenue: f64,
    /// Year-on-year, as a fraction.
    pub revenue_growth: f64,
    pub gross_margin: f64,
    pub ebitda_margin: f64,
    /// Operating margin, which is EBIT over revenue.
    pub ebit_margin: f64,
    /// Net profit (after tax) over revenue.
    pub pat_margin: f64,
    pub net_income: f64,
    pub currency: String,
}

impl Default for Fundamentals {
    fn default() -> Self {
        Fundamentals {
            market_cap: f64::NAN,
            shares: f64::NAN,
            pe: f64::NAN,
            ev_ebitda: f64::NAN,
            enterprise_value: f64::NAN,
            ebitda: f64::NAN,
            revenue: f64::NAN,
            revenue_growth: f64::NAN,
            gross_margin: f64::NAN,
            ebitda_margin: f64::NAN,
            ebit_margin: f64::NAN,
            pat_margin: f64::NAN,
            net_income: f64::NAN,
            currency: String::new(),
        }
    }
}

const SUMMARY: &str = "https://query2.finance.yahoo.com/v10/finance/quoteSummary";
const CRUMB: &str = "https://query2.finance.yahoo.com/v1/test/getcrumb";
/// Any Yahoo address hands out the session cookie, even with an error page.
const COOKIE: &str = "https://fc.yahoo.com";

/// The session (cookie, crumb), once one worked.
fn session() -> &'static Mutex<Option<(String, String)>> {
    static S: Mutex<Option<(String, String)>> = Mutex::new(None);
    &S
}

fn text(agent: &ureq::Agent, url: &str, cookie: Option<&str>) -> Result<(u16, String, Vec<String>)> {
    let mut req = agent.get(url);
    if let Some(c) = cookie {
        req = req.header("Cookie", c);
    }
    let mut resp = req
        .config()
        .http_status_as_error(false)
        .build()
        .call()
        .map_err(|_| Error::Offline)?;
    let cookies = resp
        .headers()
        .get_all("set-cookie")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .filter_map(|v| v.split(';').next())
        .map(str::to_string)
        .collect();
    let status = resp.status().as_u16();
    let body = resp
        .body_mut()
        .with_config()
        .limit(8 * 1024 * 1024)
        .read_to_string()
        .unwrap_or_default();
    Ok((status, body, cookies))
}

fn new_session(agent: &ureq::Agent) -> Result<(String, String)> {
    let (_, _, cookies) = text(agent, COOKIE, None)?;
    if cookies.is_empty() {
        return Err(Error::Bad("Yahoo didn't start a session.".into()));
    }
    let cookie = cookies.join("; ");
    let (status, crumb, _) = text(agent, CRUMB, Some(&cookie))?;
    let crumb = crumb.trim().to_string();
    if status == 429 {
        return Err(Error::Bad("Yahoo is limiting requests; try again in a minute.".into()));
    }
    if status != 200 || crumb.is_empty() || crumb.contains(' ') || crumb.len() > 64 {
        return Err(Error::Bad("Yahoo didn't start a session.".into()));
    }
    Ok((cookie, crumb))
}

/// A number from Yahoo's `{"raw": …, "fmt": …}` or a bare number.
fn raw(v: &serde_json::Value) -> f64 {
    v.get("raw").unwrap_or(v).as_f64().unwrap_or(f64::NAN)
}

pub(crate) fn parse(v: &serde_json::Value) -> Result<Fundamentals> {
    let r = &v["quoteSummary"]["result"][0];
    if r.is_null() {
        let why = v["quoteSummary"]["error"]["description"]
            .as_str()
            .unwrap_or("No fundamentals for this symbol.");
        return Err(Error::Bad(why.to_string()));
    }
    let (price, sd, ks, fd) = (
        &r["price"],
        &r["summaryDetail"],
        &r["defaultKeyStatistics"],
        &r["financialData"],
    );
    let market_cap = raw(&price["marketCap"]);
    let mut shares = raw(&ks["sharesOutstanding"]);
    if !shares.is_finite() {
        shares = raw(&ks["impliedSharesOutstanding"]);
    }
    let last = raw(&price["regularMarketPrice"]);
    if !shares.is_finite() && market_cap.is_finite() && last > 0.0 {
        shares = market_cap / last;
    }
    let mut pe = raw(&sd["trailingPE"]);
    if !pe.is_finite() {
        pe = raw(&ks["trailingPE"]);
    }
    Ok(Fundamentals {
        market_cap,
        shares,
        pe,
        ev_ebitda: raw(&ks["enterpriseToEbitda"]),
        enterprise_value: raw(&ks["enterpriseValue"]),
        ebitda: raw(&fd["ebitda"]),
        revenue: raw(&fd["totalRevenue"]),
        revenue_growth: raw(&fd["revenueGrowth"]),
        gross_margin: raw(&fd["grossMargins"]),
        ebitda_margin: raw(&fd["ebitdaMargins"]),
        ebit_margin: raw(&fd["operatingMargins"]),
        pat_margin: raw(&fd["profitMargins"]),
        net_income: raw(&ks["netIncomeToCommon"]),
        currency: price["currency"].as_str().unwrap_or_default().to_string(),
    })
}

/// One symbol's fundamentals.
pub fn fetch(symbol: &Symbol, demo: bool) -> Result<Fundamentals> {
    if demo {
        return Ok(made_up(symbol));
    }
    if matches!(symbol.kind, AssetKind::Crypto | AssetKind::Forex) {
        return Err(Error::Bad("No company fundamentals for this market.".into()));
    }
    let agent = crate::provider::agent();
    let ticker = crate::yahoo::ticker(&symbol.id);
    let url_for = |crumb: &str| {
        format!(
            "{SUMMARY}/{}?modules=price,summaryDetail,defaultKeyStatistics,financialData&crumb={}",
            crate::yahoo::urlencode(&ticker),
            crate::yahoo::urlencode(crumb)
        )
    };
    for attempt in 0..2 {
        let held = session().lock().ok().and_then(|s| s.clone());
        let (cookie, crumb) = match held {
            Some(s) if attempt == 0 => s,
            _ => {
                let s = new_session(&agent)?;
                if let Ok(mut g) = session().lock() {
                    *g = Some(s.clone());
                }
                s
            }
        };
        let (status, body, _) = text(&agent, &url_for(&crumb), Some(&cookie))?;
        match status {
            // A stale session: start a new one, once.
            401 | 403 if attempt == 0 => continue,
            429 => return Err(Error::Bad("Yahoo is limiting requests; try again in a minute.".into())),
            _ => {}
        }
        let v: serde_json::Value =
            serde_json::from_str(&body).map_err(|_| Error::Bad(format!("Yahoo answered with an error ({status}).")))?;
        return parse(&v);
    }
    Err(Error::Bad("Yahoo didn't start a session.".into()))
}

/// Steady, plausible numbers per symbol for demo mode.
fn made_up(symbol: &Symbol) -> Fundamentals {
    let mut h: u64 = 1469598103934665603;
    for b in symbol.id.bytes() {
        h = (h ^ b as u64).wrapping_mul(1099511628211);
    }
    let mut next = || {
        h ^= h << 13;
        h ^= h >> 7;
        h ^= h << 17;
        (h % 10_000) as f64 / 10_000.0
    };
    let shares = 2e8 + next() * 3e9;
    let revenue = 5e10 + next() * 9e11;
    let gross = 0.18 + next() * 0.4;
    let ebitda_m = gross * (0.35 + next() * 0.4);
    let ebit_m = ebitda_m * (0.6 + next() * 0.3);
    let pat_m = ebit_m * (0.55 + next() * 0.3);
    let pe = 12.0 + next() * 45.0;
    let net = revenue * pat_m;
    let market_cap = net * pe;
    let ebitda = revenue * ebitda_m;
    let ev = market_cap * (0.9 + next() * 0.3);
    Fundamentals {
        market_cap,
        shares,
        pe,
        ev_ebitda: ev / ebitda,
        enterprise_value: ev,
        ebitda,
        revenue,
        revenue_growth: -0.05 + next() * 0.3,
        gross_margin: gross,
        ebitda_margin: ebitda_m,
        ebit_margin: ebit_m,
        pat_margin: pat_m,
        net_income: net,
        currency: symbol.currency.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_quote_summary() {
        let v: serde_json::Value = serde_json::from_str(
            r#"{"quoteSummary":{"result":[{
                "price":{"marketCap":{"raw":4.0e12},"regularMarketPrice":{"raw":12500.0},"currency":"INR"},
                "summaryDetail":{"trailingPE":{"raw":27.5}},
                "defaultKeyStatistics":{"sharesOutstanding":{"raw":3.2e8},"enterpriseToEbitda":{"raw":18.2},
                    "enterpriseValue":{"raw":3.9e12},"netIncomeToCommon":{"raw":1.45e11}},
                "financialData":{"totalRevenue":{"raw":1.5e12},"ebitda":{"raw":2.1e11},"revenueGrowth":{"raw":0.081},
                    "grossMargins":{"raw":0.27},"ebitdaMargins":{"raw":0.14},"operatingMargins":{"raw":0.11},
                    "profitMargins":{"raw":0.097}}
            }],"error":null}}"#,
        )
        .unwrap();
        let f = parse(&v).unwrap();
        assert_eq!(f.shares, 3.2e8);
        assert_eq!(f.pe, 27.5);
        assert_eq!(f.ebit_margin, 0.11);
        assert_eq!(f.currency, "INR");
        let none: serde_json::Value =
            serde_json::from_str(r#"{"quoteSummary":{"result":null,"error":{"description":"Quote not found"}}}"#)
                .unwrap();
        assert!(parse(&none).is_err());
    }

    #[test]
    fn demo_numbers_hold_together() {
        let f = made_up(&Symbol::bare("NSE:MARUTI"));
        assert_eq!(f, made_up(&Symbol::bare("NSE:MARUTI")));
        assert!(f.gross_margin > f.ebitda_margin && f.ebitda_margin > f.ebit_margin && f.ebit_margin > f.pat_margin);
        assert!((f.market_cap / f.net_income - f.pe).abs() < 1e-6);
    }
}

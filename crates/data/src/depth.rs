//! Order books for the DOM widget: the best bids and asks from each crypto
//! exchange's public REST endpoint, polled about once a second while the
//! widget is open. Stock and forex providers don't publish books for free,
//! so those say so plainly. In demo mode the book is made up around the
//! last price.

use crate::provider::{self, Error, Result};
use opentern_core::Symbol;

/// Price levels, best first: (price, size).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Book {
    pub bids: Vec<(f64, f64)>,
    pub asks: Vec<(f64, f64)>,
}

fn levels(v: &serde_json::Value) -> Vec<(f64, f64)> {
    v.as_array()
        .map(|rows| {
            rows.iter()
                .filter_map(|r| {
                    let p = provider::num(r.get(0)?)?;
                    let q = provider::num(r.get(1)?)?;
                    (p > 0.0).then_some((p, q))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Parses a Binance-style `{bids: [[p, q]…], asks: [[p, q]…]}`.
fn parse_pairs(v: &serde_json::Value) -> Book {
    Book {
        bids: levels(&v["bids"]),
        asks: levels(&v["asks"]),
    }
}

fn parse_coinbase(v: &serde_json::Value) -> Book {
    // Rows are [price, size, orders].
    parse_pairs(v)
}

fn parse_kraken(v: &serde_json::Value) -> Result<Book> {
    if let Some(e) = v["error"].as_array().and_then(|e| e.first()) {
        return Err(Error::Bad(format!("Kraken: {}", e.as_str().unwrap_or("error"))));
    }
    let book = v["result"]
        .as_object()
        .and_then(|m| m.values().next())
        .ok_or_else(|| Error::Bad("Kraken: unexpected reply".into()))?;
    Ok(parse_pairs(book))
}

/// A made-up book around `mid`: sizes grow away from the spread.
pub fn demo_book(mid: f64, tick: f64) -> Book {
    let tick = if tick > 0.0 { tick } else { 0.01 };
    let step = (mid * 0.0002 / tick).round().max(1.0) * tick;
    let size = |k: usize| 0.5 + ((k as f64 * 1.9).sin().abs() * 3.0) + k as f64 * 0.15;
    Book {
        bids: (1..=25).map(|k| (mid - step * k as f64, size(k))).collect(),
        asks: (1..=25).map(|k| (mid + step * k as f64, size(k + 7))).collect(),
    }
}

/// The book for `sym` (`last` and `demo` for the made-up one).
pub fn book(sym: &Symbol, last: f64, demo: bool) -> Result<Book> {
    if demo {
        return Ok(demo_book(last, sym.tick_size));
    }
    let agent = provider::agent();
    let t = sym.ticker();
    let mut b = match sym.exchange() {
        "BINANCE" => parse_pairs(&provider::get_json(
            &agent,
            &format!("https://api.binance.com/api/v3/depth?symbol={t}&limit=50"),
        )?),
        "BYBIT" => {
            let v = provider::get_json(
                &agent,
                &format!("https://api.bybit.com/v5/market/orderbook?category=spot&symbol={t}&limit=50"),
            )?;
            Book {
                bids: levels(&v["result"]["b"]),
                asks: levels(&v["result"]["a"]),
            }
        }
        "OKX" => {
            let id = crate::okx::inst_id(sym);
            let v = crate::okx::Okx::default().get(&format!("/api/v5/market/books?instId={id}&sz=50"))?;
            parse_pairs(&v["data"][0])
        }
        "COINBASE" => {
            let id = crate::coinbase::product(sym);
            parse_coinbase(&provider::get_json(
                &agent,
                &format!("https://api.exchange.coinbase.com/products/{id}/book?level=2"),
            )?)
        }
        "KRAKEN" => {
            let (pair, _) = crate::kraken::pairs(sym);
            parse_kraken(&provider::get_json(
                &agent,
                &format!("https://api.kraken.com/0/public/Depth?pair={pair}&count=50"),
            )?)?
        }
        _ => {
            return Err(Error::Bad(
                "There's no free order book for this market. The DOM works for crypto exchanges.".into(),
            ));
        }
    };
    b.bids.truncate(50);
    b.asks.truncate(50);
    Ok(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_exchange_books() {
        let v: serde_json::Value = serde_json::from_str(
            r#"{"lastUpdateId":1,"bids":[["100.5","2.0"],["100.4","1.5"]],"asks":[["100.6","0.7"]]}"#,
        )
        .unwrap();
        let b = parse_pairs(&v);
        assert_eq!(b.bids, vec![(100.5, 2.0), (100.4, 1.5)]);
        assert_eq!(b.asks, vec![(100.6, 0.7)]);
        let k: serde_json::Value = serde_json::from_str(
            r#"{"error":[],"result":{"XXBTZUSD":{"bids":[["60000.1","0.3",1700000000]],"asks":[["60000.2","0.1",1700000000]]}}}"#,
        )
        .unwrap();
        assert_eq!(parse_kraken(&k).unwrap().asks, vec![(60000.2, 0.1)]);
        let d = demo_book(100.0, 0.01);
        assert!(d.bids[0].0 < 100.0 && d.asks[0].0 > 100.0);
        assert!(d.bids.windows(2).all(|w| w[0].0 > w[1].0));
    }
}

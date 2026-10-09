//! Made-up prices for `--demo`, tests and screenshots: no network, the same
//! history every time for a given symbol and interval, and a live bar that
//! ticks twice a second.

use crate::provider::{Live, Provider, Result, now_ms};
use opentern_core::{Bar, Symbol, Timeframe};
use std::time::Duration;

#[derive(Default)]
pub struct Demo;

struct Rng(u64);

impl Rng {
    fn new(seed: &str) -> Rng {
        // FNV-1a, so each symbol gets its own path.
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for b in seed.bytes() {
            h = (h ^ b as u64).wrapping_mul(0x0100_0000_01b3);
        }
        Rng(h | 1)
    }

    /// Uniform in [0, 1).
    fn next(&mut self) -> f64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 11) as f64 / (1u64 << 53) as f64
    }

    /// Roughly normal, mean 0, sd 1.
    fn normal(&mut self) -> f64 {
        (0..6).map(|_| self.next()).sum::<f64>() - 3.0
    }
}

fn start_price(sym: &Symbol) -> f64 {
    match sym.ticker() {
        t if t.starts_with("BTC") => 60_000.0,
        t if t.starts_with("ETH") => 3_000.0,
        t if t.starts_with("SOL") => 150.0,
        "EURUSD" => 1.08,
        "NIFTY" => 22_000.0,
        "RELIANCE" => 2_900.0,
        _ => 180.0,
    }
}

fn step(tf: Timeframe) -> i64 {
    tf.millis().unwrap_or(30 * 86_400_000)
}

/// Bars generated per series; every request is a slice of the same path,
/// so history, paging and the live bar all agree.
const DEPTH: usize = 20_000;

/// `n` bars ending with the one forming now.
fn history(sym: &Symbol, tf: Timeframe, n: usize) -> Vec<Bar> {
    let mut all = path(sym, tf);
    let n = n.min(all.len());
    all.split_off(all.len() - n)
}

fn path(sym: &Symbol, tf: Timeframe) -> Vec<Bar> {
    // Longer intervals need fewer bars to cover decades.
    let n = match tf.unit {
        opentern_core::Unit::Day => DEPTH / 4,
        opentern_core::Unit::Week => DEPTH / 20,
        opentern_core::Unit::Month => DEPTH / 80,
        _ => DEPTH,
    };
    let len = step(tf);
    let now = now_ms();
    let last = now - now.rem_euclid(len);
    let mut rng = Rng::new(&format!("{}:{}", sym.id, tf.label()));
    let vol = 0.0008 * ((len as f64) / 60_000.0).sqrt().min(40.0);
    let start = start_price(sym);
    let mut price = start;
    let mut drift = 0.0;
    (0..n)
        .map(|i| {
            // A gentle pull back toward the start, so prices stay plausible.
            let pull = -(price / start).ln() * 0.01 * vol;
            drift = drift * 0.97 + rng.normal() * vol * 0.03 + pull;
            let open = price;
            let close = (open * (1.0 + drift + rng.normal() * vol)).max(open * 0.5);
            let high = open.max(close) * (1.0 + rng.next() * vol);
            let low = open.min(close) * (1.0 - rng.next() * vol);
            price = close;
            let tick = sym.tick_size.max(1e-8);
            let r = |x: f64| (x / tick).round() * tick;
            Bar {
                time: last - (n - 1 - i) as i64 * len,
                open: r(open),
                high: r(high),
                low: r(low),
                close: r(close),
                volume: (1000.0 * (1.0 + rng.next() * 4.0) * (1.0 + drift.abs() * 50.0)).round(),
            }
        })
        .collect()
}

impl Provider for Demo {
    fn natives(&self) -> Vec<Timeframe> {
        ["1m", "5m", "15m", "1h", "4h", "1D", "1W", "1M"]
            .iter()
            .filter_map(|s| Timeframe::parse(s))
            .collect()
    }

    fn latest(&self, sym: &Symbol, tf: Timeframe, n: usize) -> Result<Vec<Bar>> {
        Ok(history(sym, tf, n))
    }

    fn since(&self, sym: &Symbol, tf: Timeframe, start: i64) -> Result<Vec<Bar>> {
        Ok(history(sym, tf, 5000).into_iter().filter(|b| b.time >= start).collect())
    }

    fn before(&self, _: &Symbol, _: Timeframe, _: i64, _: usize) -> Result<Vec<Bar>> {
        Ok(Vec::new())
    }

    fn stream(&self, sym: &Symbol, tf: Timeframe, sink: &mut dyn FnMut(Live) -> bool) -> Result<()> {
        let mut rng = Rng::new(&format!("live:{}", sym.id));
        let mut bar = *path(sym, tf).last().expect("bars");
        let len = step(tf);
        loop {
            std::thread::sleep(Duration::from_millis(500));
            let now = now_ms();
            let open_time = now - now.rem_euclid(len);
            if open_time > bar.time {
                bar = Bar {
                    time: open_time,
                    open: bar.close,
                    high: bar.close,
                    low: bar.close,
                    close: bar.close,
                    volume: 0.0,
                };
            }
            let tick = sym.tick_size.max(1e-8);
            let price = ((bar.close * (1.0 + rng.normal() * 0.0004)) / tick).round() * tick;
            bar.close = price;
            bar.high = bar.high.max(price);
            bar.low = bar.low.min(price);
            bar.volume += (rng.next() * 20.0).round();
            if !sink(Live::Bar(bar)) {
                return Ok(());
            }
        }
    }
}

/// A made-up quote (last, previous close) for the watchlist in demo mode.
pub fn quote(sym: &Symbol) -> (f64, f64) {
    let day = Timeframe::parse("1D").expect("valid");
    let bars = history(sym, day, 2);
    (bars[1].close, bars[0].close)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_history_each_time_and_well_formed() {
        let sym = opentern_core::starter_watchlist().remove(0);
        let tf = Timeframe::parse("1h").unwrap();
        let a = history(&sym, tf, 500);
        let b = history(&sym, tf, 500);
        assert_eq!(a.len(), 500);
        assert_eq!(a[..10], b[..10]);
        for w in a.windows(2) {
            assert_eq!(w[1].time - w[0].time, 3_600_000);
        }
        for x in &a {
            assert!(x.low <= x.open.min(x.close) && x.high >= x.open.max(x.close));
        }
    }
}

//! Builds bars from trades or price ticks, for sources that stream prices
//! rather than candles (Coinbase matches, OANDA prices, broker quotes).
//! Buckets match the resampler's: intraday bars start on the local day's
//! grid, daily and longer bars are stamped at 00:00 UTC of the local date.

use crate::resample::{calendar_bucket, to_utc_midnight};
use jiff::tz::TimeZone;
use opentern_core::{Bar, Timeframe};

pub struct BarBuilder {
    tf: Timeframe,
    tz: TimeZone,
    /// The forming bar.
    pub bar: Option<Bar>,
}

impl BarBuilder {
    /// `last` is the newest bar from history, so a stream continues it.
    pub fn new(tf: Timeframe, tz: TimeZone, last: Option<Bar>) -> BarBuilder {
        BarBuilder { tf, tz, bar: last }
    }

    /// The open time of the bar that a trade at `t` belongs to.
    pub fn bucket(&self, t: i64) -> i64 {
        match self.tf.millis() {
            Some(ms) if self.tf.is_intraday() => {
                let off = jiff::Timestamp::from_millisecond(t)
                    .map(|ts| ts.to_zoned(self.tz.clone()).offset().seconds() as i64 * 1000)
                    .unwrap_or(0);
                (t + off).div_euclid(ms) * ms - off
            }
            _ => calendar_bucket(to_utc_midnight(t, &self.tz), self.tf),
        }
    }

    /// Folds a trade in and returns the forming bar. `size` is the traded
    /// quantity (0 when the source gives prices only).
    pub fn trade(&mut self, t: i64, price: f64, size: f64) -> Bar {
        let b = self.bucket(t);
        match &mut self.bar {
            Some(bar) if bar.time == b => {
                bar.high = bar.high.max(price);
                bar.low = bar.low.min(price);
                bar.close = price;
                bar.volume += size;
            }
            // A late trade for an older bar is folded into the current one's
            // close only if it's newer; otherwise ignored.
            Some(bar) if b < bar.time => {}
            _ => {
                self.bar = Some(Bar {
                    time: b,
                    open: price,
                    high: price,
                    low: price,
                    close: price,
                    volume: size,
                })
            }
        }
        self.bar.expect("a bar")
    }

    /// Sets the day's running volume (sources that report cumulative
    /// volume): the bar's volume grows by the difference.
    pub fn add_volume(&mut self, v: f64) {
        if let Some(b) = &mut self.bar
            && v > 0.0
        {
            b.volume += v;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use opentern_core::Unit;

    #[test]
    fn builds_minute_bars_from_trades() {
        let mut b = BarBuilder::new(Timeframe::new(1, Unit::Minute), TimeZone::UTC, None);
        b.trade(60_000, 10.0, 1.0);
        b.trade(70_000, 12.0, 2.0);
        let bar = b.trade(110_000, 9.0, 1.0);
        assert_eq!(
            (bar.time, bar.open, bar.high, bar.low, bar.close, bar.volume),
            (60_000, 10.0, 12.0, 9.0, 9.0, 4.0)
        );
        let next = b.trade(125_000, 11.0, 1.0);
        assert_eq!((next.time, next.open, next.volume), (120_000, 11.0, 1.0));
        // A late trade for the previous minute doesn't reopen it.
        let still = b.trade(119_000, 50.0, 1.0);
        assert_eq!(still.time, 120_000);
        assert_eq!(still.high, 11.0);
    }

    #[test]
    fn hourly_bars_follow_the_local_grid_and_days_are_utc_stamped() {
        // India is UTC+5:30: hourly bars start at :30 UTC.
        let ist = TimeZone::get("Asia/Kolkata").unwrap();
        let b = BarBuilder::new(Timeframe::new(1, Unit::Hour), ist.clone(), None);
        assert_eq!(b.bucket(3_600_000 * 10 + 60_000).rem_euclid(3_600_000), 1_800_000);
        let d = BarBuilder::new(Timeframe::new(1, Unit::Day), ist, None);
        // 2023-11-14 22:00 UTC is already the 15th in India.
        let t = 1_700_000_000_000;
        assert_eq!(d.bucket(t), 1_700_006_400_000);
    }
}

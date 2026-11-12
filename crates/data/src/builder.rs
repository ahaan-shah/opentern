//! Builds bars from trades or price ticks, for sources that stream prices
//! rather than candles (Coinbase matches, OANDA prices, broker quotes).
//! Intraday bars continue the grid of the history they follow (stocks'
//! hourly bars start at the session open, 09:30 or 09:15, not on the
//! hour), else the local clock's; daily and longer bars are stamped at
//! 00:00 UTC of the local date, as the resampler's.

use crate::resample::{calendar_bucket, to_utc_midnight};
use jiff::tz::TimeZone;
use opentern_core::{Bar, Timeframe};

pub struct BarBuilder {
    tf: Timeframe,
    tz: TimeZone,
    /// Where intraday bars start within the local clock's grid (ms past
    /// it): 30 minutes for a US stock's hourly bars from 09:30.
    phase: i64,
    /// The forming bar.
    pub bar: Option<Bar>,
}

impl BarBuilder {
    /// `last` is the newest bar from history, so a stream continues it and
    /// its grid.
    pub fn new(tf: Timeframe, tz: TimeZone, last: Option<Bar>) -> BarBuilder {
        let phase = match (tf.millis(), last) {
            (Some(ms), Some(b)) if tf.is_intraday() => (b.time + offset(b.time, &tz)).rem_euclid(ms),
            _ => 0,
        };
        BarBuilder {
            tf,
            tz,
            phase,
            bar: last,
        }
    }

    /// The open time of the bar that a trade at `t` belongs to.
    pub fn bucket(&self, t: i64) -> i64 {
        match self.tf.millis() {
            Some(ms) if self.tf.is_intraday() => {
                let off = offset(t, &self.tz);
                (t + off - self.phase).div_euclid(ms) * ms + self.phase - off
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

/// `tz`'s UTC offset at `t`, in ms.
fn offset(t: i64, tz: &TimeZone) -> i64 {
    jiff::Timestamp::from_millisecond(t)
        .map(|ts| ts.to_zoned(tz.clone()).offset().seconds() as i64 * 1000)
        .unwrap_or(0)
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
    fn live_bars_continue_the_history_grid_across_a_clock_change() {
        // A US stock's hourly history starts at 09:30 New York. The last
        // bar is Friday 6 Mar 2026 15:30 (20:30 UTC, EST); Monday's trades
        // (EDT, clocks forward) still land on :30 bars, from 09:30.
        let ny = TimeZone::get("America/New_York").unwrap();
        let ms = |s: &str| s.parse::<jiff::Timestamp>().unwrap().as_millisecond();
        let last = Bar {
            time: ms("2026-03-06T20:30:00Z"),
            open: 1.0,
            high: 1.0,
            low: 1.0,
            close: 1.0,
            volume: 1.0,
        };
        let mut b = BarBuilder::new(Timeframe::new(1, Unit::Hour), ny.clone(), Some(last));
        assert_eq!(b.bucket(ms("2026-03-06T20:59:00Z")), last.time);
        assert_eq!(
            b.trade(ms("2026-03-09T13:30:05Z"), 2.0, 1.0).time,
            ms("2026-03-09T13:30:00Z")
        );
        assert_eq!(b.bucket(ms("2026-03-09T14:29:59Z")), ms("2026-03-09T13:30:00Z"));
        assert_eq!(b.bucket(ms("2026-03-09T14:30:00Z")), ms("2026-03-09T14:30:00Z"));
        // NSE's from 09:15 IST.
        let ist = TimeZone::get("Asia/Kolkata").unwrap();
        let nse = Bar {
            time: ms("2026-11-11T03:45:00Z"),
            ..last
        };
        let b = BarBuilder::new(Timeframe::new(1, Unit::Hour), ist, Some(nse));
        assert_eq!(b.bucket(ms("2026-11-12T05:00:00Z")), ms("2026-11-12T04:45:00Z"));
        // No history: the clock's grid.
        let b = BarBuilder::new(Timeframe::new(1, Unit::Hour), ny, None);
        assert_eq!(b.bucket(ms("2026-03-09T13:45:00Z")), ms("2026-03-09T13:00:00Z"));
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

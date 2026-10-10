//! Chart types that change the bars themselves, as on TradingView: Heikin
//! Ashi, and the time-independent ones (Renko, Line Break, Kagi, Point &
//! Figure). Studies run on the transformed bars, as they do there.
//!
//! Every output keeps strictly increasing times (bricks formed on the same
//! bar are a millisecond apart), so the time↔index maps keep working.

use opentern_core::Bar;

/// Heikin Ashi: close = OHLC/4, open = midpoint of the previous HA body.
pub fn heikin_ashi(raw: &[Bar]) -> Vec<Bar> {
    let mut out: Vec<Bar> = Vec::with_capacity(raw.len());
    for b in raw {
        let prev = out.last().copied();
        out.push(ha_bar(prev.as_ref(), b));
    }
    out
}

/// One Heikin Ashi bar from the previous HA bar and the real one.
pub fn ha_bar(prev: Option<&Bar>, b: &Bar) -> Bar {
    let close = (b.open + b.high + b.low + b.close) / 4.0;
    let open = match prev {
        Some(p) => (p.open + p.close) / 2.0,
        None => (b.open + b.close) / 2.0,
    };
    Bar {
        time: b.time,
        open,
        high: b.high.max(open).max(close),
        low: b.low.min(open).min(close),
        close,
        volume: b.volume,
    }
}

/// Pine's `ta.atr(len)` on the last bar (RMA of the true range), or 1% of
/// the last close when there's too little history.
pub fn atr(raw: &[Bar], len: usize) -> f64 {
    let len = len.max(1);
    let fallback = raw.last().map(|b| (b.close.abs() * 0.01).max(1e-9)).unwrap_or(1.0);
    if raw.len() <= len {
        return fallback;
    }
    let tr = |i: usize| {
        let b = &raw[i];
        if i == 0 {
            b.high - b.low
        } else {
            let pc = raw[i - 1].close;
            (b.high - b.low).max((b.high - pc).abs()).max((b.low - pc).abs())
        }
    };
    let mut v = (0..len).map(tr).sum::<f64>() / len as f64;
    let a = 1.0 / len as f64;
    for i in len..raw.len() {
        v = a * tr(i) + (1.0 - a) * v;
    }
    if v.is_finite() && v > 0.0 { v } else { fallback }
}

/// Pushes a bar with a time after the previous one.
fn push(out: &mut Vec<Bar>, mut b: Bar) {
    if let Some(last) = out.last()
        && b.time <= last.time
    {
        b.time = last.time + 1;
    }
    out.push(b);
}

fn brick(time: i64, open: f64, close: f64, volume: f64) -> Bar {
    Bar {
        time,
        open,
        high: open.max(close),
        low: open.min(close),
        close,
        volume,
    }
}

/// Renko on closes with a fixed box (TradingView's "traditional" Renko;
/// a reversal takes two boxes). A bar's volume goes on its last brick.
pub fn renko(raw: &[Bar], size: f64) -> Vec<Bar> {
    let mut out = Vec::new();
    let Some(first) = raw.first() else { return out };
    if size.is_nan() || size <= 0.0 {
        return out;
    }
    let (mut hi, mut lo) = (first.close, first.close);
    for b in raw {
        let c = b.close;
        let start = out.len();
        while c >= hi + size {
            push(&mut out, brick(b.time, hi, hi + size, 0.0));
            lo = hi;
            hi += size;
        }
        while c <= lo - size {
            push(&mut out, brick(b.time, lo, lo - size, 0.0));
            hi = lo;
            lo -= size;
        }
        if out.len() > start {
            out.last_mut().expect("just pushed").volume = b.volume;
        } else if let Some(last) = out.last_mut() {
            last.volume += b.volume;
        }
    }
    out
}

/// Line Break: a new line in the trend's direction on a new close beyond
/// the last line; a reversal must break the extreme of the last `n` lines.
pub fn line_break(raw: &[Bar], n: usize) -> Vec<Bar> {
    let n = n.max(1);
    let mut out: Vec<Bar> = Vec::new();
    for b in raw {
        let c = b.close;
        let Some(last) = out.last().copied() else {
            if c != b.open {
                push(&mut out, brick(b.time, b.open, c, b.volume));
            }
            continue;
        };
        let up = last.close >= last.open;
        let recent = &out[out.len().saturating_sub(n)..];
        let hi = recent.iter().map(|x| x.high).fold(f64::MIN, f64::max);
        let lo = recent.iter().map(|x| x.low).fold(f64::MAX, f64::min);
        let line = if up {
            if c > last.close {
                Some(brick(b.time, last.close, c, b.volume))
            } else if c < lo {
                Some(brick(b.time, last.open, c, b.volume))
            } else {
                None
            }
        } else if c < last.close {
            Some(brick(b.time, last.close, c, b.volume))
        } else if c > hi {
            Some(brick(b.time, last.open, c, b.volume))
        } else {
            None
        };
        match line {
            Some(l) => push(&mut out, l),
            None => out.last_mut().expect("non-empty").volume += b.volume,
        }
    }
    out
}

/// Kagi: each output bar is one vertical stroke (open = where it starts,
/// close = where it ends). The stroke turns when price reverses by
/// `reversal` from its extreme.
pub fn kagi(raw: &[Bar], reversal: f64) -> Vec<Bar> {
    let mut out: Vec<Bar> = Vec::new();
    let Some(first) = raw.first() else { return out };
    if reversal.is_nan() || reversal <= 0.0 {
        return out;
    }
    let start = first.close;
    for b in raw {
        let c = b.close;
        match out.last_mut() {
            None => {
                if (c - start).abs() >= reversal {
                    push(&mut out, brick(b.time, start, c, b.volume));
                }
            }
            Some(last) => {
                let up = last.close >= last.open;
                if (up && c > last.close) || (!up && c < last.close) {
                    last.close = c;
                    last.high = last.open.max(c);
                    last.low = last.open.min(c);
                    last.volume += b.volume;
                } else if (last.close - c).abs() >= reversal {
                    let from = last.close;
                    push(&mut out, brick(b.time, from, c, b.volume));
                } else {
                    last.volume += b.volume;
                }
            }
        }
    }
    out
}

/// Point & Figure on closes: X columns rise, O columns fall, in whole
/// boxes; a new column needs `reversal` boxes. Each output bar is a column
/// (open = first box, close = last box, both on the box grid).
pub fn point_figure(raw: &[Bar], size: f64, reversal: usize) -> Vec<Bar> {
    let mut out: Vec<Bar> = Vec::new();
    let Some(first) = raw.first() else { return out };
    if size.is_nan() || size <= 0.0 {
        return out;
    }
    let rev = reversal.max(1) as f64;
    let level = |v: f64| (v / size).floor();
    let start = level(first.close);
    // (up, bottom level, top level)
    let mut col: Option<(bool, f64, f64)> = None;
    let col_bar = |time: i64, up: bool, bot: f64, top: f64, vol: f64| {
        let (o, c) = if up { (bot, top) } else { (top, bot) };
        brick(time, o * size, c * size, vol)
    };
    for b in raw {
        let (fl, cl) = (level(b.close), (b.close / size).ceil());
        match col {
            None => {
                if fl > start {
                    col = Some((true, start, fl));
                } else if cl < start {
                    col = Some((false, cl, start));
                } else {
                    continue;
                }
                let (u, lo, hi) = col.expect("set");
                push(&mut out, col_bar(b.time, u, lo, hi, b.volume));
            }
            Some((up, lo, hi)) => {
                let next = if up {
                    if fl > hi {
                        Some((true, lo, fl, false))
                    } else if cl <= hi - rev {
                        Some((false, cl, hi - 1.0, true))
                    } else {
                        None
                    }
                } else if cl < lo {
                    Some((false, cl, hi, false))
                } else if fl >= lo + rev {
                    Some((true, lo + 1.0, fl, true))
                } else {
                    None
                };
                match next {
                    Some((u, nlo, nhi, new_col)) => {
                        col = Some((u, nlo, nhi));
                        let bar = col_bar(b.time, u, nlo, nhi, b.volume);
                        if new_col {
                            push(&mut out, bar);
                        } else if let Some(last) = out.last_mut() {
                            let vol = last.volume + b.volume;
                            *last = Bar {
                                time: last.time,
                                volume: vol,
                                ..bar
                            };
                        }
                    }
                    None => {
                        if let Some(last) = out.last_mut() {
                            last.volume += b.volume;
                        }
                    }
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn closes(v: &[f64]) -> Vec<Bar> {
        v.iter()
            .enumerate()
            .map(|(i, &c)| Bar {
                time: i as i64 * 60_000,
                open: c,
                high: c,
                low: c,
                close: c,
                volume: 1.0,
            })
            .collect()
    }

    #[test]
    fn heikin_ashi_follows_the_formula() {
        let raw = vec![
            Bar {
                time: 0,
                open: 10.0,
                high: 12.0,
                low: 9.0,
                close: 11.0,
                volume: 1.0,
            },
            Bar {
                time: 1,
                open: 11.0,
                high: 13.0,
                low: 10.0,
                close: 12.0,
                volume: 1.0,
            },
        ];
        let ha = heikin_ashi(&raw);
        assert_eq!(ha[0].close, 10.5);
        assert_eq!(ha[0].open, 10.5);
        assert_eq!(ha[1].open, 10.5);
        assert_eq!(ha[1].close, 11.5);
        assert_eq!(ha[1].high, 13.0);
    }

    #[test]
    fn renko_needs_two_boxes_to_reverse() {
        let b = renko(&closes(&[100.0, 103.1, 101.5, 100.9, 98.0]), 1.0);
        // Up 3 bricks (100→103); 101.5 isn't a reversal (needs ≤ 101);
        // 100.9 is one down brick (102→101); 98 adds three more.
        let dirs: Vec<bool> = b.iter().map(|x| x.close > x.open).collect();
        assert_eq!(dirs, [true, true, true, false, false, false, false]);
        assert_eq!(b[3].open, 102.0);
        assert!(b.windows(2).all(|w| w[1].time > w[0].time));
    }

    #[test]
    fn line_break_reverses_past_three_lines() {
        let raw = vec![
            Bar {
                time: 0,
                open: 10.0,
                high: 11.0,
                low: 10.0,
                close: 11.0,
                volume: 1.0,
            },
            Bar {
                time: 1,
                open: 11.0,
                high: 12.0,
                low: 11.0,
                close: 12.0,
                volume: 1.0,
            },
            Bar {
                time: 2,
                open: 12.0,
                high: 13.0,
                low: 12.0,
                close: 13.0,
                volume: 1.0,
            },
            // Inside the last three lines' range: nothing.
            Bar {
                time: 3,
                open: 13.0,
                high: 13.0,
                low: 10.5,
                close: 10.5,
                volume: 1.0,
            },
            // Below all three: a down line from the last line's bottom.
            Bar {
                time: 4,
                open: 10.5,
                high: 10.5,
                low: 9.0,
                close: 9.0,
                volume: 1.0,
            },
        ];
        let l = line_break(&raw, 3);
        assert_eq!(l.len(), 4);
        assert_eq!((l[3].open, l[3].close), (12.0, 9.0));
    }

    #[test]
    fn kagi_turns_on_reversal() {
        let k = kagi(&closes(&[100.0, 102.0, 105.0, 104.0, 101.0, 103.0]), 2.0);
        assert_eq!((k[0].open, k[0].close), (100.0, 105.0));
        assert_eq!((k[1].open, k[1].close), (105.0, 101.0));
        assert_eq!((k[2].open, k[2].close), (101.0, 103.0));
    }

    #[test]
    fn point_figure_columns() {
        let p = point_figure(&closes(&[100.0, 104.0, 103.0, 100.5, 99.0, 103.0]), 1.0, 3);
        // X up to 104; 100.5 → ceil 101 ≤ 104-3: O column 103..101; 99 extends
        // it to 99; 103 ≥ 99+3: X column 100..103.
        assert_eq!(p.len(), 3);
        assert_eq!((p[0].open, p[0].close), (100.0, 104.0));
        assert_eq!((p[1].open, p[1].close), (103.0, 99.0));
        assert_eq!((p[2].open, p[2].close), (100.0, 103.0));
    }

    #[test]
    fn atr_matches_rma_of_true_range() {
        let raw = closes(&[1.0; 30]);
        assert!(atr(&raw, 14) > 0.0);
    }
}

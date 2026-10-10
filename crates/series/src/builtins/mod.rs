//! TradingView's built-in indicators, ported from their Pine source line by
//! line onto the `ta` primitives, with TradingView's default inputs,
//! colours, levels and fills. `docs/indicators.md` tracks each one and what
//! has been checked against TradingView's own output.

mod bands;
mod drawn;
mod ma;
mod osc;
mod trend;
mod volume;
mod wave2;

use crate::color::Rgba;
use crate::study::*;
use std::sync::OnceLock;

/// Every built-in, in the order the Indicators dialog lists them.
pub fn all() -> &'static [Meta] {
    static ALL: OnceLock<Vec<Meta>> = OnceLock::new();
    ALL.get_or_init(|| {
        let mut v = Vec::new();
        v.extend(ma::metas());
        v.extend(bands::metas());
        v.extend(trend::metas());
        v.extend(osc::metas());
        v.extend(volume::metas());
        v.extend(wave2::metas());
        v.extend(drawn::metas());
        v.sort_by_key(|a| a.name.to_lowercase());
        v
    })
}

pub fn by_id(id: &str) -> Option<&'static Meta> {
    all().iter().find(|m| m.id == id)
}

// ------------------------------------------------------------ declaration helpers

pub(crate) fn int(id: &'static str, title: &'static str, default: i64, min: i64, max: i64) -> InputSpec {
    InputSpec {
        id,
        title,
        kind: InputKind::Int { min, max },
        default: Value::Int(default),
        group: None,
    }
}

pub(crate) fn length(default: i64) -> InputSpec {
    int("length", "Length", default, 1, 5000)
}

pub(crate) fn float(id: &'static str, title: &'static str, default: f64, min: f64, max: f64, step: f64) -> InputSpec {
    InputSpec {
        id,
        title,
        kind: InputKind::Float { min, max, step },
        default: Value::Float(default),
        group: None,
    }
}

pub(crate) fn source(default: &str) -> InputSpec {
    InputSpec {
        id: "source",
        title: "Source",
        kind: InputKind::Source,
        default: Value::Text(default.into()),
        group: None,
    }
}

pub(crate) fn choice(
    id: &'static str,
    title: &'static str,
    default: &str,
    options: &'static [&'static str],
) -> InputSpec {
    InputSpec {
        id,
        title,
        kind: InputKind::Choice(options),
        default: Value::Text(default.into()),
        group: None,
    }
}

pub(crate) fn flag(id: &'static str, title: &'static str, default: bool) -> InputSpec {
    InputSpec {
        id,
        title,
        kind: InputKind::Bool,
        default: Value::Bool(default),
        group: None,
    }
}

pub(crate) fn offset_input() -> InputSpec {
    int("offset", "Offset", 0, -500, 500)
}

impl InputSpec {
    pub(crate) fn group(mut self, g: &'static str) -> Self {
        self.group = Some(g);
        self
    }
}

pub(crate) fn line(title: &'static str, color: Rgba) -> PlotSpec {
    PlotSpec {
        title,
        style: PlotStyle::Line,
        colors: vec![("Color", color)],
        width: 1,
        offset: Offset::Fixed(0),
        visible: true,
        base: 0.0,
        shape: None,
    }
}

impl PlotSpec {
    pub(crate) fn style(mut self, s: PlotStyle) -> Self {
        self.style = s;
        self
    }
    pub(crate) fn width(mut self, w: u8) -> Self {
        self.width = w;
        self
    }
    pub(crate) fn colors(mut self, c: &[(&'static str, Rgba)]) -> Self {
        self.colors = c.to_vec();
        self
    }
    pub(crate) fn offset_by(mut self, id: &'static str, sign: i64, add: i64) -> Self {
        self.offset = Offset::Input { id, sign, add };
        self
    }
    pub(crate) fn hidden(mut self) -> Self {
        self.visible = false;
        self
    }
}

pub(crate) fn hline(title: &'static str, value: f64, color: Rgba) -> HLineSpec {
    HLineSpec {
        title,
        value,
        color,
        style: LineStyle::Dashed,
        width: 1,
        visible: true,
    }
}

impl HLineSpec {
    pub(crate) fn style(mut self, s: LineStyle) -> Self {
        self.style = s;
        self
    }
}

/// TradingView's level colour (`#787B86`) and its half-transparent middle.
pub(crate) const LEVEL: Rgba = Rgba::hex(0x787B86);
pub(crate) const MID: Rgba = Rgba::hex(0x787B86).transp(50);

pub(crate) fn fill_h(title: &'static str, a: usize, b: usize, color: Rgba) -> FillSpec {
    FillSpec {
        title,
        between: FillBetween::HLines(a, b),
        colors: vec![("Color", color)],
        visible: true,
    }
}

pub(crate) fn fill_p(title: &'static str, a: usize, b: usize, color: Rgba) -> FillSpec {
    FillSpec {
        title,
        between: FillBetween::Plots(a, b),
        colors: vec![("Color", color)],
        visible: true,
    }
}

impl FillSpec {
    pub(crate) fn colors(mut self, c: &[(&'static str, Rgba)]) -> Self {
        self.colors = c.to_vec();
        self
    }
}

/// The three dashed bands of a bounded oscillator, with the fill between
/// the outer two: 70/50/30 for RSI, 80/50/20 for Stochastic.
pub(crate) fn bands(upper: f64, middle: f64, lower: f64, fill: Rgba) -> (Vec<HLineSpec>, Vec<FillSpec>) {
    (
        vec![
            hline("Upper Band", upper, LEVEL),
            hline("Middle Band", middle, MID),
            hline("Lower Band", lower, LEVEL),
        ],
        vec![fill_h("Background", 0, 2, fill)],
    )
}

pub(crate) fn zero_line() -> HLineSpec {
    hline("Zero Line", 0.0, MID)
}

#[cfg(test)]
mod tests {
    use super::*;
    use opentern_core::{Bar, Timeframe};

    pub(crate) fn bars(n: usize) -> Vec<Bar> {
        (0..n)
            .map(|i| {
                let x = i as f64;
                let c = 100.0 + (x * 0.21).sin() * 6.0 + (x * 0.031).cos() * 15.0 + x * 0.02;
                let o = c - (x * 0.7).sin() * 1.5;
                Bar {
                    time: i as i64 * 3_600_000,
                    open: o,
                    high: o.max(c) + 0.5 + (x * 1.3).sin().abs(),
                    low: o.min(c) - 0.5 - (x * 0.9).cos().abs(),
                    close: c,
                    volume: 1000.0 + (x * 0.5).sin().abs() * 500.0,
                }
            })
            .collect()
    }

    #[test]
    fn ids_unique_and_specs_consistent() {
        let all = all();
        assert!(all.len() >= 60, "only {} built-ins", all.len());
        let mut ids: Vec<_> = all.iter().map(|m| m.id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), all.len(), "duplicate ids");
        for m in all {
            for f in &m.fills {
                match f.between {
                    FillBetween::Plots(a, b) => assert!(a < m.plots.len() && b < m.plots.len(), "{}", m.id),
                    FillBetween::HLines(a, b) => assert!(a < m.hlines.len() && b < m.hlines.len(), "{}", m.id),
                }
            }
            for p in &m.plots {
                assert!(!p.colors.is_empty(), "{}: {}", m.id, p.title);
                if let Offset::Input { id, .. } = p.offset {
                    assert!(m.input(id).is_some(), "{}: offset input {id}", m.id);
                }
            }
        }
    }

    /// Every built-in runs on every bar without panicking, produces numbers
    /// (not just na) once warmed up, and gives the same result whether the
    /// last bar is computed as forming or as closed.
    #[test]
    fn every_builtin_runs_and_realtime_matches_history() {
        let b = bars(600);
        let tf = Timeframe::parse("1h").unwrap();
        let tz = jiff::tz::TimeZone::UTC;
        let sess = sessions(&b, tf, &tz);
        for m in all() {
            let mut full = Instance::new(m, &m.default_inputs());
            full.recompute(&b, &sess, tf);
            // Plots, a background (Trading Sessions has only that) or
            // drawings (Zig Zag and friends).
            let any = full.out.plots.iter().any(|p| p.iter().skip(300).any(|v| v.is_finite()))
                || full.out.bg.iter().any(|c| *c != 0)
                || full.drawings.is_some();
            assert!(any, "{} produced nothing", m.id);
            // Bar by bar, with the last bar ticking three times first.
            let mut live = Instance::new(m, &m.default_inputs());
            live.recompute(&b[..400], &sess[..400], tf);
            for n in 401..=b.len() {
                let mut partial = b[n - 1];
                partial.close = partial.open;
                partial.high = partial.open.max(partial.close);
                partial.low = partial.open.min(partial.close);
                let mut tmp = b[..n].to_vec();
                tmp[n - 1] = partial;
                live.update(&tmp, &sess[..n], tf);
                live.update(&b[..n], &sess[..n], tf);
            }
            for (p, (a, c)) in full.out.plots.iter().zip(&live.out.plots).enumerate() {
                for i in 0..b.len() {
                    let (x, y) = (a[i], c[i]);
                    assert!(
                        (x.is_nan() && y.is_nan()) || (x - y).abs() <= 1e-9 * x.abs().max(1.0),
                        "{} plot {p} bar {i}: {x} vs {y}",
                        m.id
                    );
                }
            }
            assert_eq!(full.drawings, live.drawings, "{} drawings", m.id);
        }
    }
}

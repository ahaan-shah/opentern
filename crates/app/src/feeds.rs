//! Other symbols' and timeframes' bars for Pine `request.security`. A
//! script that asks for a series it doesn't have gets `na` and lists it
//! (`PineStudy::wanted`); this fetches it into the shared `Feeds`
//! (`opentern_pine::Env::feeds`, one store for every chart) and reruns the
//! scripts. Feeds in use are refreshed once a minute, so realtime values
//! keep moving.

use crate::app::App;
use opentern_core::{Bar, Symbol, Timeframe};
use opentern_pine::PineStudy;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Fetch sequence numbers for feeds start here (the screener and the
/// Strategy Lab use lower ranges).
pub const SEQ_BASE: u64 = 1 << 42;
const REFRESH: Duration = Duration::from_secs(60);

pub struct FeedReq {
    pub symbol: String,
    pub tf: Timeframe,
    seq: u64,
    asked: Instant,
    /// When a script last asked for it (refreshes stop for unused feeds).
    used: Instant,
    pending: bool,
}

#[derive(Default)]
pub struct FeedState {
    pub reqs: HashMap<String, FeedReq>,
    next_seq: u64,
}

/// How many bars of `tf` cover the chart's history, with some warm-up.
fn bars_needed(chart: &[Bar], tf: Timeframe) -> usize {
    let span = match (chart.first(), chart.last()) {
        (Some(a), Some(b)) => (b.time - a.time).max(0),
        _ => 0,
    };
    let ms = tf.millis().unwrap_or(30 * 86_400_000).max(1);
    ((span / ms) as usize + 300).clamp(300, 20_000)
}

impl App {
    /// The symbol a script's string means: a full id, or a bare ticker
    /// resolved against the watchlists (as formulas are).
    fn feed_symbol(&self, text: &str) -> Symbol {
        let mut known: Vec<Symbol> = opentern_core::starter_watchlist();
        for w in &self.watchlists {
            known.extend(w.symbols.iter().cloned());
        }
        known.push(self.chart.symbol.clone());
        let id = opentern_data::expr::resolve(text.trim(), &known);
        known.into_iter().find(|s| s.id == id).unwrap_or_else(|| {
            let mut s = Symbol::bare(&id);
            s.timezone = opentern_data::expr::guess_tz(&id);
            s
        })
    }

    /// Asks for what this chart's scripts want; refreshes feeds in use.
    pub(crate) fn request_feeds(&mut self) {
        let mut wanted: Vec<(String, Timeframe)> = Vec::new();
        for s in &self.chart.studies {
            if let Some(p) = s.inst.latest_state().and_then(|a| a.downcast_ref::<PineStudy>()) {
                wanted.extend(p.wanted().iter().cloned());
            }
        }
        // Feeds already loaded aren't "wanted" any more; keep them fresh
        // while a script on this chart uses them.
        let now = Instant::now();
        if self.scripts.iter().any(|r| r.pine.is_some()) {
            for r in self.feeds_state.reqs.values_mut() {
                r.used = now;
            }
        }
        let count_for = |tf| bars_needed(&self.chart.bars, tf);
        let mut sends = Vec::new();
        for (sym, tf) in wanted {
            let key = opentern_pine::feed_key(&sym, tf);
            if self.feeds_state.reqs.contains_key(&key) {
                continue;
            }
            let st = &mut self.feeds_state;
            st.next_seq += 1;
            let seq = SEQ_BASE + st.next_seq;
            st.reqs.insert(
                key,
                FeedReq {
                    symbol: sym.clone(),
                    tf,
                    seq,
                    asked: now,
                    used: now,
                    pending: true,
                },
            );
            sends.push((seq, sym, tf, count_for(tf)));
        }
        // Refresh feeds that are in use and a minute old.
        for r in self.feeds_state.reqs.values_mut() {
            if !r.pending && now - r.asked > REFRESH && now - r.used < REFRESH * 2 {
                r.asked = now;
                r.pending = true;
                sends.push((r.seq, r.symbol.clone(), r.tf, count_for(r.tf)));
            }
        }
        for (seq, sym, tf, count) in sends {
            let symbol = self.feed_symbol(&sym);
            self.hub.send(opentern_data::Request::Fetch { seq, symbol, tf, count });
        }
    }

    /// A feed arrived: store it and rerun every chart's Pine scripts.
    pub(crate) fn feed_fetched(&mut self, seq: u64, result: Result<Vec<Bar>, String>) {
        let Some((key, req)) = self.feeds_state.reqs.iter_mut().find(|(_, r)| r.seq == seq) else {
            return;
        };
        req.pending = false;
        let bars = match result {
            Ok(b) => b,
            Err(e) => {
                crate::diag::crumb(format!("couldn't load {key} for a script: {e}"));
                Vec::new()
            }
        };
        let key = key.clone();
        let changed = {
            let mut m = self.feeds.write().unwrap_or_else(|e| e.into_inner());
            let same = m.get(&key).is_some_and(|old| **old == bars);
            if !same {
                m.insert(key, Arc::new(bars));
            }
            !same
        };
        if changed {
            self.each_chart(|a| a.rerun_pine());
        }
    }

    /// Recomputes the Pine studies on this chart (new feed data).
    fn rerun_pine(&mut self) {
        let uids: Vec<u64> = self
            .scripts
            .iter()
            .filter(|r| r.pine.is_some())
            .filter_map(|r| r.uid)
            .collect();
        for uid in uids {
            if let Some(ins) = self
                .chart
                .studies
                .iter()
                .find(|s| s.uid == uid)
                .map(|s| s.inst.inputs.clone())
            {
                self.chart.set_inputs(uid, &ins);
            }
        }
    }
}

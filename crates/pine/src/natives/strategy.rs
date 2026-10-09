//! `strategy.*`: orders go to the broker emulator (`opentern_engine`), which
//! fills them the way TradingView's does; the variables read it back.

use super::{NatDef, arg, d, int_or, str_or};
use crate::run::G;
use crate::value::V;
use opentern_engine::{Broker, OcaType, Order, OrderKind, Trade};

type R = Result<V, String>;

fn broker(g: &mut G) -> Result<&mut Broker, String> {
    g.broker
        .as_mut()
        .ok_or_else(|| "strategy.* needs strategy(…) instead of indicator(…) at the top.".to_string())
}

fn opt(v: &V) -> Option<f64> {
    let f = v.f();
    (!f.is_nan()).then_some(f)
}

fn long_of(v: &V) -> Result<bool, String> {
    match v {
        V::Str(s) if s.as_ref() == "long" => Ok(true),
        V::Str(s) if s.as_ref() == "short" => Ok(false),
        V::Bool(b) => Ok(*b),
        _ => Err("The direction must be strategy.long or strategy.short.".into()),
    }
}

fn oca(name: &V, kind: &V) -> (String, OcaType) {
    let name = name.as_str().unwrap_or("").to_string();
    let kind = match kind.as_str() {
        Some("strategy.oca.cancel") => OcaType::Cancel,
        Some("strategy.oca.reduce") => OcaType::Reduce,
        _ => OcaType::None,
    };
    (name, kind)
}

/// `when = false` (v4/v5) skips the call.
fn skipped(a: &[V], k: usize) -> bool {
    matches!(arg(a, k), V::Bool(false))
}

fn text(a: &[V], k: usize) -> String {
    str_or(a, k, "").to_string()
}

fn entry_like(g: &mut G, a: &[V], kind: OrderKind) -> R {
    // id, direction, qty, limit, stop, oca_name, oca_type, comment, …, when
    if skipped(a, 10) {
        return Ok(V::Na);
    }
    let id = str_or(a, 0, "").to_string();
    let long = long_of(arg(a, 1))?;
    let mut o = Order::market(&id, kind, long);
    o.qty = opt(arg(a, 2)).filter(|q| *q > 0.0);
    o.limit = opt(arg(a, 3));
    o.stop = opt(arg(a, 4));
    (o.oca_name, o.oca_type) = oca(arg(a, 5), arg(a, 6));
    o.comment = text(a, 7);
    if o.comment.is_empty() {
        o.comment = id;
    }
    broker(g)?.submit(o);
    Ok(V::Na)
}

fn closed(g: &mut G, a: &[V], f: impl Fn(&Trade) -> V) -> R {
    let k = int_or(a, 0, -1);
    let b = broker(g)?;
    Ok(usize::try_from(k)
        .ok()
        .and_then(|k| b.trades.get(k))
        .map(f)
        .unwrap_or(V::Na))
}

fn open_lot(g: &mut G, a: &[V], f: impl Fn(&opentern_engine::Lot, f64) -> V) -> R {
    let k = int_or(a, 0, -1);
    let close = g.bar().close;
    let b = broker(g)?;
    Ok(usize::try_from(k)
        .ok()
        .and_then(|k| b.lots.get(k))
        .map(|l| f(l, close))
        .unwrap_or(V::Na))
}

fn sum_pnl(b: &Broker, keep: impl Fn(f64) -> bool) -> f64 {
    b.trades.iter().map(|t| t.pnl).filter(|p| keep(*p)).sum()
}

fn avg(xs: impl Iterator<Item = f64>) -> V {
    let v: Vec<f64> = xs.collect();
    if v.is_empty() {
        V::Na
    } else {
        V::Float(v.iter().sum::<f64>() / v.len() as f64)
    }
}

fn pct_of_entry(t: &Trade) -> f64 {
    let base = t.entry_price * t.qty;
    if base == 0.0 { f64::NAN } else { t.pnl / base * 100.0 }
}

const ENTRY: &[&str] = &[
    "id",
    "direction",
    "qty",
    "limit",
    "stop",
    "oca_name",
    "oca_type",
    "comment",
    "alert_message",
    "disable_alert",
    "when",
];

pub(crate) fn defs() -> Vec<NatDef> {
    vec![
        d("strategy.entry", ENTRY, |g, a, _| entry_like(g, a, OrderKind::Entry)),
        d("strategy.order", ENTRY, |g, a, _| entry_like(g, a, OrderKind::Order)),
        d(
            "strategy.exit",
            &[
                "id",
                "from_entry",
                "qty",
                "qty_percent",
                "profit",
                "limit",
                "loss",
                "stop",
                "trail_price",
                "trail_points",
                "trail_offset",
                "oca_name",
                "comment",
                "comment_profit",
                "comment_loss",
                "comment_trailing",
                "alert_message",
                "alert_profit",
                "alert_loss",
                "alert_trailing",
                "disable_alert",
                "when",
            ],
            |g, a, _| {
                if skipped(a, 21) {
                    return Ok(V::Na);
                }
                let id = str_or(a, 0, "").to_string();
                // The side is taken from the position when it fills.
                let mut o = Order::market(&id, OrderKind::Exit, true);
                o.from_entry = text(a, 1);
                o.qty = opt(arg(a, 2)).filter(|q| *q > 0.0);
                o.qty_percent = opt(arg(a, 3)).unwrap_or(100.0).clamp(0.0, 100.0);
                o.profit_ticks = opt(arg(a, 4));
                o.limit = opt(arg(a, 5));
                o.loss_ticks = opt(arg(a, 6));
                o.stop = opt(arg(a, 7));
                o.trail_price = opt(arg(a, 8));
                o.trail_points = opt(arg(a, 9));
                o.trail_offset = opt(arg(a, 10));
                o.oca_name = text(a, 11);
                o.comment = text(a, 12);
                o.comment_profit = text(a, 13);
                o.comment_loss = text(a, 14);
                o.comment_trailing = text(a, 15);
                if o.comment.is_empty() {
                    o.comment = id;
                }
                let b = broker(g)?;
                // Keep the trailing stop's progress when the same exit is
                // placed again on the next bar.
                if let Some(old) = b.pending.iter().find(|p| p.id == o.id && p.kind == OrderKind::Exit) {
                    o.trail_best = old.trail_best;
                }
                let has_level = o.profit_ticks.is_some()
                    || o.limit.is_some()
                    || o.loss_ticks.is_some()
                    || o.stop.is_some()
                    || (o.trail_offset.is_some() && (o.trail_points.is_some() || o.trail_price.is_some()));
                if has_level {
                    b.submit(o);
                }
                Ok(V::Na)
            },
        ),
        d(
            "strategy.close",
            &[
                "id",
                "comment",
                "qty",
                "qty_percent",
                "alert_message",
                "immediately",
                "disable_alert",
                "when",
            ],
            |g, a, _| {
                if skipped(a, 7) {
                    return Ok(V::Na);
                }
                let id = str_or(a, 0, "").to_string();
                let b = broker(g)?;
                // Nothing to close: TradingView ignores it.
                if !b.lots.iter().any(|l| l.id == id) {
                    return Ok(V::Na);
                }
                let mut o = Order::market(&id, OrderKind::Close, true);
                o.comment = text(a, 1);
                o.qty = opt(arg(a, 2)).filter(|q| *q > 0.0);
                o.qty_percent = opt(arg(a, 3)).unwrap_or(100.0).clamp(0.0, 100.0);
                o.immediately = arg(a, 5).truthy();
                b.submit(o);
                Ok(V::Na)
            },
        ),
        d(
            "strategy.close_all",
            &["comment", "alert_message", "immediately", "disable_alert", "when"],
            |g, a, _| {
                if skipped(a, 4) {
                    return Ok(V::Na);
                }
                let b = broker(g)?;
                if b.lots.is_empty() {
                    return Ok(V::Na);
                }
                let mut o = Order::market("", OrderKind::Close, true);
                o.comment = text(a, 0);
                o.immediately = arg(a, 2).truthy();
                b.submit(o);
                Ok(V::Na)
            },
        ),
        d("strategy.cancel", &["id", "when"], |g, a, _| {
            if !skipped(a, 1) {
                let id = str_or(a, 0, "").to_string();
                broker(g)?.cancel(&id);
            }
            Ok(V::Na)
        }),
        d("strategy.cancel_all", &["when"], |g, a, _| {
            if !skipped(a, 0) {
                broker(g)?.cancel_all();
            }
            Ok(V::Na)
        }),
        d("strategy.risk.allow_entry_in", &["value"], |g, a, _| {
            let v = match arg(a, 0).as_str() {
                Some("strategy.direction.long") => Some(true),
                Some("strategy.direction.short") => Some(false),
                _ => None,
            };
            broker(g)?.props.allow_entry_in = v;
            Ok(V::Na)
        }),
        // Accepted; not enforced yet (see CLAUDE.md, Phase 7).
        d(
            "strategy.risk.max_drawdown",
            &["value", "type", "alert_message"],
            |_, _, _| Ok(V::Na),
        ),
        d(
            "strategy.risk.max_intraday_loss",
            &["value", "type", "alert_message"],
            |_, _, _| Ok(V::Na),
        ),
        d(
            "strategy.risk.max_cons_loss_days",
            &["count", "alert_message"],
            |_, _, _| Ok(V::Na),
        ),
        d(
            "strategy.risk.max_intraday_filled_orders",
            &["count", "alert_message"],
            |_, _, _| Ok(V::Na),
        ),
        d("strategy.risk.max_position_size", &["contracts"], |_, _, _| Ok(V::Na)),
        d("strategy.convert_to_account", &["value"], |_, a, _| {
            Ok(arg(a, 0).clone())
        }),
        d(
            "strategy.convert_to_symbol",
            &["value"],
            |_, a, _| Ok(arg(a, 0).clone()),
        ),
        d("strategy.default_entry_qty", &["fill_price"], |g, a, _| {
            let p = arg(a, 0).f();
            let b = broker(g)?;
            let v = b.props.qty_value;
            Ok(V::Float(match b.props.qty_type {
                opentern_engine::QtyType::Fixed => v,
                opentern_engine::QtyType::Cash => v / p,
                opentern_engine::QtyType::PercentOfEquity => b.equity_at(p) * v / 100.0 / p,
            }))
        }),
        // ---------------------------------------------------------- variables
        d("strategy.position_size", &[], |g, _, _| {
            Ok(V::Float(broker(g)?.position_size()))
        })
        .var(),
        d("strategy.position_avg_price", &[], |g, _, _| {
            let p = broker(g)?.position_avg_price();
            Ok(if p.is_nan() { V::Na } else { V::Float(p) })
        })
        .var(),
        d("strategy.position_entry_name", &[], |g, _, _| {
            Ok(V::str(broker(g)?.lots.last().map(|l| l.id.as_str()).unwrap_or("")))
        })
        .var(),
        d("strategy.equity", &[], |g, _, _| {
            let c = g.bar().close;
            Ok(V::Float(broker(g)?.equity_at(c)))
        })
        .var(),
        d("strategy.initial_capital", &[], |g, _, _| {
            Ok(V::Float(broker(g)?.props.initial_capital))
        })
        .var(),
        d("strategy.netprofit", &[], |g, _, _| Ok(V::Float(broker(g)?.net_profit))).var(),
        d("strategy.netprofit_percent", &[], |g, _, _| {
            let b = broker(g)?;
            Ok(V::Float(b.net_profit / b.props.initial_capital * 100.0))
        })
        .var(),
        d("strategy.openprofit", &[], |g, _, _| {
            let c = g.bar().close;
            Ok(V::Float(broker(g)?.open_profit(c)))
        })
        .var(),
        d("strategy.openprofit_percent", &[], |g, _, _| {
            let c = g.bar().close;
            let b = broker(g)?;
            let cost: f64 = b.lots.iter().map(|l| l.qty * l.price).sum();
            Ok(V::Float(if cost == 0.0 {
                0.0
            } else {
                b.open_profit(c) / cost * 100.0
            }))
        })
        .var(),
        d("strategy.grossprofit", &[], |g, _, _| {
            Ok(V::Float(sum_pnl(broker(g)?, |p| p > 0.0)))
        })
        .var(),
        d("strategy.grossloss", &[], |g, _, _| {
            Ok(V::Float(-sum_pnl(broker(g)?, |p| p < 0.0)))
        })
        .var(),
        d("strategy.closedtrades", &[], |g, _, _| {
            Ok(V::Int(broker(g)?.trades.len() as i64))
        })
        .var(),
        d("strategy.opentrades", &[], |g, _, _| {
            Ok(V::Int(broker(g)?.lots.len() as i64))
        })
        .var(),
        d("strategy.wintrades", &[], |g, _, _| {
            Ok(V::Int(broker(g)?.wins() as i64))
        })
        .var(),
        d("strategy.losstrades", &[], |g, _, _| {
            Ok(V::Int(broker(g)?.losses() as i64))
        })
        .var(),
        d("strategy.eventrades", &[], |g, _, _| {
            Ok(V::Int(broker(g)?.trades.iter().filter(|t| t.pnl == 0.0).count() as i64))
        })
        .var(),
        d("strategy.max_drawdown", &[], |g, _, _| {
            Ok(V::Float(broker(g)?.max_drawdown))
        })
        .var(),
        d("strategy.max_runup", &[], |g, _, _| Ok(V::Float(broker(g)?.max_runup))).var(),
        d("strategy.max_contracts_held_all", &[], |g, _, _| {
            Ok(V::Float(
                broker(g)?.position.iter().map(|p| p.abs()).fold(0.0, f64::max),
            ))
        })
        .var(),
        d("strategy.max_contracts_held_long", &[], |g, _, _| {
            Ok(V::Float(broker(g)?.position.iter().cloned().fold(0.0, f64::max)))
        })
        .var(),
        d("strategy.max_contracts_held_short", &[], |g, _, _| {
            Ok(V::Float(-broker(g)?.position.iter().cloned().fold(0.0, f64::min)))
        })
        .var(),
        d("strategy.account_currency", &[], |g, _, _| {
            Ok(V::str(g.env.currency.as_str()))
        })
        .var(),
        d("strategy.avg_trade", &[], |g, _, _| {
            Ok(avg(broker(g)?.trades.iter().map(|t| t.pnl)))
        })
        .var(),
        d("strategy.avg_winning_trade", &[], |g, _, _| {
            Ok(avg(broker(g)?.trades.iter().map(|t| t.pnl).filter(|p| *p > 0.0)))
        })
        .var(),
        d("strategy.avg_losing_trade", &[], |g, _, _| {
            Ok(avg(broker(g)?.trades.iter().map(|t| t.pnl).filter(|p| *p < 0.0)))
        })
        .var(),
        d("strategy.avg_trade_percent", &[], |g, _, _| {
            Ok(avg(broker(g)?.trades.iter().map(pct_of_entry)))
        })
        .var(),
        // ---------------------------------------------------------- closed trades
        d("strategy.closedtrades.entry_price", &["trade_num"], |g, a, _| {
            closed(g, a, |t| V::Float(t.entry_price))
        }),
        d("strategy.closedtrades.exit_price", &["trade_num"], |g, a, _| {
            closed(g, a, |t| V::Float(t.exit_price))
        }),
        d("strategy.closedtrades.entry_bar_index", &["trade_num"], |g, a, _| {
            closed(g, a, |t| V::Int(t.entry_bar as i64))
        }),
        d("strategy.closedtrades.exit_bar_index", &["trade_num"], |g, a, _| {
            closed(g, a, |t| V::Int(t.exit_bar as i64))
        }),
        d("strategy.closedtrades.entry_time", &["trade_num"], |g, a, _| {
            closed(g, a, |t| V::Int(t.entry_time))
        }),
        d("strategy.closedtrades.exit_time", &["trade_num"], |g, a, _| {
            closed(g, a, |t| V::Int(t.exit_time))
        }),
        d("strategy.closedtrades.entry_id", &["trade_num"], |g, a, _| {
            closed(g, a, |t| V::str(t.entry_id.as_str()))
        }),
        d("strategy.closedtrades.exit_id", &["trade_num"], |g, a, _| {
            closed(g, a, |t| V::str(t.exit_id.as_str()))
        }),
        d("strategy.closedtrades.entry_comment", &["trade_num"], |g, a, _| {
            closed(g, a, |t| V::str(t.entry_comment.as_str()))
        }),
        d("strategy.closedtrades.exit_comment", &["trade_num"], |g, a, _| {
            closed(g, a, |t| V::str(t.exit_comment.as_str()))
        }),
        d("strategy.closedtrades.profit", &["trade_num"], |g, a, _| {
            closed(g, a, |t| V::Float(t.pnl))
        }),
        d("strategy.closedtrades.profit_percent", &["trade_num"], |g, a, _| {
            closed(g, a, |t| V::Float(pct_of_entry(t)))
        }),
        d("strategy.closedtrades.commission", &["trade_num"], |g, a, _| {
            closed(g, a, |t| V::Float(t.commission))
        }),
        d("strategy.closedtrades.size", &["trade_num"], |g, a, _| {
            closed(g, a, |t| V::Float(if t.long { t.qty } else { -t.qty }))
        }),
        d("strategy.closedtrades.max_runup", &["trade_num"], |g, a, _| {
            closed(g, a, |t| V::Float(t.runup))
        }),
        d("strategy.closedtrades.max_drawdown", &["trade_num"], |g, a, _| {
            closed(g, a, |t| V::Float(-t.drawdown))
        }),
        d("strategy.closedtrades.max_runup_percent", &["trade_num"], |g, a, _| {
            closed(g, a, |t| V::Float(t.runup / (t.entry_price * t.qty) * 100.0))
        }),
        d(
            "strategy.closedtrades.max_drawdown_percent",
            &["trade_num"],
            |g, a, _| closed(g, a, |t| V::Float(-t.drawdown / (t.entry_price * t.qty) * 100.0)),
        ),
        d("strategy.closedtrades.first_index", &[], |_, _, _| Ok(V::Int(0))).var(),
        // ---------------------------------------------------------- open trades
        d("strategy.opentrades.entry_price", &["trade_num"], |g, a, _| {
            open_lot(g, a, |l, _| V::Float(l.price))
        }),
        d("strategy.opentrades.entry_bar_index", &["trade_num"], |g, a, _| {
            open_lot(g, a, |l, _| V::Int(l.bar as i64))
        }),
        d("strategy.opentrades.entry_time", &["trade_num"], |g, a, _| {
            open_lot(g, a, |l, _| V::Int(l.time))
        }),
        d("strategy.opentrades.entry_id", &["trade_num"], |g, a, _| {
            open_lot(g, a, |l, _| V::str(l.id.as_str()))
        }),
        d("strategy.opentrades.entry_comment", &["trade_num"], |g, a, _| {
            open_lot(g, a, |l, _| V::str(l.comment.as_str()))
        }),
        d("strategy.opentrades.size", &["trade_num"], |g, a, _| {
            open_lot(g, a, |l, _| V::Float(if l.long { l.qty } else { -l.qty }))
        }),
        d("strategy.opentrades.profit", &["trade_num"], |g, a, _| {
            open_lot(g, a, |l, c| {
                V::Float((c - l.price) * l.qty * if l.long { 1.0 } else { -1.0 } - l.commission)
            })
        }),
        d("strategy.opentrades.profit_percent", &["trade_num"], |g, a, _| {
            open_lot(g, a, |l, c| {
                V::Float((c / l.price - 1.0) * 100.0 * if l.long { 1.0 } else { -1.0 })
            })
        }),
        d("strategy.opentrades.commission", &["trade_num"], |g, a, _| {
            open_lot(g, a, |l, _| V::Float(l.commission))
        }),
        d("strategy.opentrades.max_runup", &["trade_num"], |g, a, _| {
            open_lot(g, a, |l, _| {
                let best = if l.long { l.hi } else { l.lo };
                V::Float(((best - l.price) * l.qty * if l.long { 1.0 } else { -1.0 }).max(0.0))
            })
        }),
        d("strategy.opentrades.max_drawdown", &["trade_num"], |g, a, _| {
            open_lot(g, a, |l, _| {
                let worst = if l.long { l.lo } else { l.hi };
                V::Float(-((worst - l.price) * l.qty * if l.long { 1.0 } else { -1.0 }).min(0.0))
            })
        }),
    ]
}

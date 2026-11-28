use opentern_core::{Bar, Timeframe, Unit};
use opentern_pine::{Env, compile};
use opentern_series::study::Inputs;

fn bars() -> [Bar; 3] {
    [
        (100.0, 100.0, 100.0, 100.0),
        (100.0, 110.0, 80.0, 105.0),
        (105.0, 105.0, 105.0, 105.0),
    ]
    .map(|(open, high, low, close)| Bar {
        time: 0,
        open,
        high,
        low,
        close,
        volume: 1.0,
    })
}

#[test]
fn market_fill_arms_same_bar_exit() {
    let script = compile(
        r#"//@version=6
strategy("COOF chain", calc_on_order_fills=true)
if bar_index == 0
    strategy.entry("L1", strategy.long)
if strategy.opentrades == 1
    strategy.exit("TP", "L1", limit=108)
"#,
        "order_fills.pine",
        Env::default(),
    )
    .unwrap();
    let mut run = script.study(&Inputs::default());
    run.run(&bars(), Timeframe::new(1, Unit::Day));
    assert!(run.error().is_none(), "{:?}", run.error());
    let broker = run.broker().unwrap();
    assert_eq!(
        broker.trades.len(),
        1,
        "the market fill must arm TP before price reaches 108"
    );
    assert!(run.logs().is_empty());
}

fn run(source: &str, bars: &[Bar]) -> (opentern_pine::PineStudy, opentern_series::study::Output) {
    let script = compile(source, "order_fills.pine", Env::default()).unwrap();
    let mut run = script.study(&Inputs::default());
    let out = run.run(bars, Timeframe::new(1, Unit::Day));
    assert!(run.error().is_none(), "{:?}", run.error());
    (run, out)
}

#[test]
fn omitted_and_disabled_coof_keep_one_execution_per_bar() {
    let source = r#"//@version=6
strategy("COOF off", calc_on_order_fills=false)
if bar_index == 0
    strategy.entry("L1", strategy.long)
if strategy.opentrades == 1
    strategy.exit("TP", "L1", limit=108)
log.info(str.tostring(strategy.opentrades))
plot(strategy.position_size)
"#;
    let (off, out) = run(source, &bars());
    let (omitted, omitted_out) = run(&source.replace(", calc_on_order_fills=false", ""), &bars());
    assert_eq!(off.broker(), omitted.broker());
    assert_eq!(out.plots, omitted_out.plots);
    assert_eq!(out.plots[0], [0.0, 1.0, 1.0]);
    assert_eq!(off.logs(), omitted.logs());
    assert_eq!(off.logs().len(), 3);
    let b = off.broker().unwrap();
    assert_eq!(b.fills.len(), 1);
    assert_eq!(b.trades.len(), 0);
    assert_eq!(b.equity, [1_000_000.0, 1_000_005.0, 1_000_005.0]);
}

#[test]
fn limit_and_exit_callbacks_see_trades_and_only_the_remaining_path() {
    let source = r#"//@version=6
strategy("COOF path", calc_on_order_fills=true, pyramiding=2)
if bar_index == 0
    strategy.entry("L1", strategy.short)
if strategy.opentrades == 1 and strategy.closedtrades == 0
    strategy.entry("L2", strategy.short, limit=106)
if strategy.opentrades == 2 and strategy.opentrades.entry_id(1) == "L2"
    strategy.exit("TP", "L2", limit=95)
if strategy.closedtrades == 1 and strategy.closedtrades.exit_id(0) == "TP"
    strategy.entry("passed-high", strategy.short, limit=108)
"#;
    let any = source.replace("pyramiding=2", r#"pyramiding=2, close_entries_rule="ANY""#);
    for (source, closed_entry) in [(source, "L1"), (any.as_str(), "L2")] {
        let (run, _) = run(source, &bars());
        let b = run.broker().unwrap();
        let fills: Vec<_> = b.fills.iter().map(|f| (f.id.as_str(), f.bar, f.price)).collect();
        assert_eq!(fills, [("L1", 1, 100.0), ("L2", 1, 106.0), ("TP", 1, 95.0)]);
        assert_eq!(b.trades.len(), 1);
        assert_eq!(b.trades[0].entry_id, closed_entry);
        assert_eq!(b.equity.len(), 3);
        assert_eq!(b.equity[1], 1_000_006.0);
        assert_eq!(b.position, [0.0, -1.0, -1.0]);
        assert_eq!(b.pending[0].id, "passed-high");
    }
}

#[test]
fn same_tick_orders_fill_before_one_callback() {
    for (on_close, bar, passes) in [(false, 1, [1.0, 3.0, 4.0]), (true, 0, [2.0, 3.0, 4.0])] {
        let source = format!(
            r#"//@version=6
strategy("COOF shared tick", calc_on_order_fills=true, process_orders_on_close={on_close}, pyramiding=2)
varip int passes = 0
varip bool cancelled = false
passes += 1
if bar_index == 0 and strategy.opentrades == 0
    strategy.entry("A", strategy.long)
    strategy.entry("B", strategy.long)
if strategy.opentrades == 1 and strategy.opentrades.entry_id(0) == "A"
    cancelled := true
    strategy.cancel("B")
plot(passes)
plot(cancelled ? 1 : 0)
"#,
        );
        let (run, out) = run(&source, &bars());
        let b = run.broker().unwrap();
        let fills: Vec<_> = b.fills.iter().map(|f| (f.id.as_str(), f.bar, f.price)).collect();
        assert_eq!(fills, [("A", bar, 100.0), ("B", bar, 100.0)]);
        assert!(b.pending.is_empty());
        assert_eq!(out.plots[0], passes);
        assert_eq!(out.plots[1], [0.0; 3]);
    }
}

#[test]
fn different_prices_on_one_path_segment_have_separate_callbacks() {
    let (run, out) = run(
        r#"//@version=6
strategy("COOF distinct ticks", calc_on_order_fills=true, pyramiding=2)
varip int passes = 0
varip bool saw_one = false
passes += 1
if bar_index == 0
    strategy.entry("A", strategy.long, limit=95)
    strategy.entry("B", strategy.long, limit=90)
if strategy.opentrades == 1
    saw_one := true
plot(passes)
plot(saw_one ? 1 : 0)
"#,
        &bars(),
    );
    let b = run.broker().unwrap();
    let fills: Vec<_> = b.fills.iter().map(|f| (f.id.as_str(), f.bar, f.price)).collect();
    assert_eq!(fills, [("A", 1, 95.0), ("B", 1, 90.0)]);
    assert_eq!(out.plots[0], [1.0, 4.0, 5.0]);
    assert_eq!(out.plots[1], [0.0, 1.0, 1.0]);
}

#[test]
fn orders_created_or_replaced_by_a_group_callback_wait_for_the_next_tick() {
    let (run, out) = run(
        r#"//@version=6
strategy("COOF deferred group", calc_on_order_fills=true, pyramiding=4)
varip int passes = 0
varip bool submitted = false
passes += 1
if bar_index == 0
    strategy.entry("A", strategy.long)
    strategy.entry("B", strategy.long)
    strategy.entry("replaced", strategy.long, limit=70)
if strategy.opentrades == 2 and not submitted
    submitted := true
    strategy.entry("replaced", strategy.long, stop=100)
    strategy.entry("new", strategy.long)
plot(passes)
"#,
        &bars(),
    );
    let b = run.broker().unwrap();
    let fills: Vec<_> = b.fills.iter().map(|f| (f.id.as_str(), f.bar, f.price)).collect();
    assert_eq!(
        fills,
        [
            ("A", 1, 100.0),
            ("B", 1, 100.0),
            ("new", 1, 110.0),
            ("replaced", 1, 110.0)
        ]
    );
    assert_eq!(out.plots[0], [1.0, 4.0, 5.0]);
}

#[test]
fn oca_cancel_and_reduce_apply_within_one_tick_group() {
    for (mode, expected) in [("cancel", vec![("A", 1.0)]), ("reduce", vec![("A", 1.0), ("B", 2.0)])] {
        let source = format!(
            r#"//@version=6
strategy("COOF same tick OCA", calc_on_order_fills=true)
varip int passes = 0
passes += 1
if bar_index == 0
    strategy.order("A", strategy.long, qty=1, limit=95, oca_name="g", oca_type=strategy.oca.{mode})
    strategy.order("B", strategy.long, qty=3, limit=95, oca_name="g", oca_type=strategy.oca.{mode})
plot(passes)
"#,
        );
        let (run, out) = run(&source, &bars());
        let b = run.broker().unwrap();
        assert_eq!(
            b.fills.iter().map(|f| (f.id.as_str(), f.qty)).collect::<Vec<_>>(),
            expected
        );
        assert!(b.fills.iter().all(|f| f.bar == 1 && f.price == 95.0));
        assert!(b.pending.is_empty());
        assert_eq!(out.plots[0], [1.0, 3.0, 4.0]);
    }
}

#[test]
fn oca_cancel_and_reduce_apply_before_the_next_fill() {
    for (mode, expected) in [("cancel", vec![1.0]), ("reduce", vec![1.0, 2.0])] {
        let source = format!(
            r#"//@version=6
strategy("COOF OCA", calc_on_order_fills=true)
if bar_index == 0
    strategy.order("second", strategy.long, qty=3, limit=90, oca_name="g", oca_type=strategy.oca.{mode})
    strategy.order("first", strategy.long, qty=1, limit=95, oca_name="g", oca_type=strategy.oca.{mode})
"#
        );
        let (run, _) = run(&source, &bars());
        let b = run.broker().unwrap();
        assert_eq!(b.fills[0].id, "first");
        assert_eq!(b.fills.iter().map(|f| f.qty).collect::<Vec<_>>(), expected);
        assert!(b.pending.is_empty());
    }
}

#[test]
fn pyramiding_rejects_entry_without_an_extra_callback() {
    let (run, out) = run(
        r#"//@version=6
strategy("COOF pyramiding", calc_on_order_fills=true, pyramiding=1)
varip int passes = 0
passes += 1
if bar_index == 0
    strategy.entry("A", strategy.long)
    strategy.entry("B", strategy.long)
plot(passes)
"#,
        &bars(),
    );
    assert_eq!(run.broker().unwrap().fills.len(), 1);
    assert_eq!(out.plots[0], [1.0, 3.0, 4.0]);
}

#[test]
fn fills_keep_commission_and_slippage_and_commit_equity_once() {
    let (run, _) = run(
        r#"//@version=6
strategy("COOF costs", calc_on_order_fills=true, commission_type=strategy.commission.cash_per_order, commission_value=2, slippage=3)
if bar_index == 0
    strategy.entry("A", strategy.long)
if strategy.opentrades == 1
    strategy.exit("TP", "A", limit=108)
"#,
        &bars(),
    );
    let b = run.broker().unwrap();
    assert_eq!(b.fills.len(), 2);
    assert_eq!(b.trades.len(), 1);
    assert_eq!(b.fills[0].price, 100.03);
    assert_eq!(b.fills[1].price, 108.0);
    assert_eq!(b.commission_paid, 4.0);
    assert_eq!(b.trades[0].commission, 4.0);
    assert!((b.net_profit - 3.97).abs() < 1e-9);
    assert_eq!(b.equity, [1_000_000.0, 1_000_003.97, 1_000_003.97]);
    assert_eq!(b.equity_hi.len(), 3);
    assert_eq!(b.equity_lo.len(), 3);
}

#[test]
fn margin_liquidation_is_visible_to_its_callback() {
    let mut data = bars();
    data[1].high = 150.0;
    let (run, _) = run(
        r#"//@version=6
strategy("COOF margin", calc_on_order_fills=true, initial_capital=1000, margin_short=100)
varip bool seen = false
if bar_index == 0
    strategy.entry("A", strategy.short, qty=10)
if strategy.closedtrades == 1 and not seen and strategy.closedtrades.exit_id(0) == "Margin call"
    seen := true
    strategy.entry("after liquidation", strategy.long, qty=1)
"#,
        &data,
    );
    let b = run.broker().unwrap();
    assert_eq!(b.margin_calls, [10.0]);
    assert_eq!(b.fills[1].id, "Margin call");
    assert_eq!(b.net_profit, -500.0);
    // Position can open at the next tick, the close after the high.
    assert_eq!(b.fills[2].id, "after liquidation");
    assert_eq!(b.fills[2].bar, 1);
    assert_eq!(b.fills[2].price, 105.0);
}

#[test]
fn process_orders_on_close_recalculates_and_commits_final_output() {
    let (run, out) = run(
        r#"//@version=6
strategy("COOF at close", calc_on_order_fills=true, process_orders_on_close=true)
if bar_index == 0 and strategy.opentrades == 0
    strategy.entry("A", strategy.long)
if bar_index == 1 and strategy.opentrades == 1
    strategy.close("A")
plot(strategy.opentrades)
plot(strategy.closedtrades)
"#,
        &bars(),
    );
    let b = run.broker().unwrap();
    assert_eq!(b.fills.len(), 2);
    assert_eq!((b.fills[0].bar, b.fills[0].price), (0, 100.0));
    assert_eq!((b.fills[1].bar, b.fills[1].price), (1, 105.0));
    assert_eq!(out.plots[0], [1.0, 0.0, 0.0]);
    assert_eq!(out.plots[1], [0.0, 1.0, 1.0]);
    assert_eq!(b.equity.len(), 3);
}

#[test]
fn exit_fill_on_closing_tick_still_allows_a_closing_execution() {
    // The path visits 110 before L1 fills at 80, so 105 is first
    // reachable by the newly armed TP on the closing tick.
    let mut data = bars().to_vec();
    data[2] = Bar {
        open: 102.0,
        low: 102.0,
        ..data[2]
    };
    data.push(Bar {
        open: 103.0,
        low: 103.0,
        ..data[2]
    });
    let (run, out) = run(
        r#"//@version=6
strategy("COOF close and re-entry", calc_on_order_fills=true, process_orders_on_close=false)
varip bool saw_tp = false
varip bool submitted = false
varip int passes = 0
var int commits = 0
passes += 1
commits += 1
if bar_index == 0
    strategy.entry("L1", strategy.long, limit=80)
if strategy.opentrades == 1 and strategy.closedtrades == 0
    strategy.exit("TP", "L1", limit=105)
bool tp_callback = strategy.closedtrades == 1 and not saw_tp
if tp_callback
    saw_tp := true
if saw_tp and not tp_callback and not submitted and strategy.position_size == 0
    submitted := true
    strategy.entry("L1", strategy.long)
plot(passes)
plot(commits)
plot(passes[1])
"#,
        &data,
    );
    let b = run.broker().unwrap();
    let fills: Vec<_> = b
        .fills
        .iter()
        .map(|f| (f.id.as_str(), f.exit, f.bar, f.price))
        .collect();
    assert_eq!(
        fills,
        [("L1", false, 1, 80.0), ("TP", true, 1, 105.0), ("L1", false, 2, 102.0)]
    );
    assert_eq!(out.plots[0], [1.0, 4.0, 6.0, 7.0]);
    assert_eq!(out.plots[1], [1.0, 2.0, 3.0, 4.0]);
    assert_eq!(out.plots[2][1..], [1.0, 4.0, 6.0]);
    assert_eq!(b.equity.len(), data.len());
}

#[test]
fn rollback_keeps_varip_and_broker_and_commits_history_once() {
    let (run, out) = run(
        r#"//@version=6
strategy("COOF rollback", calc_on_order_fills=true)
type Counter
    int regular = 0
    varip int persistent = 0
var Counter counter = Counter.new()
counter.regular += 1
counter.persistent += 1
f_passes() =>
    var int regular = 0
    varip int persistent = 0
    regular += 1
    persistent += 1
    [regular, persistent, persistent[1]]
[regular, persistent, previous] = f_passes()
var array<int> rolled = array.new_int()
varip array<int> retained = array.new_int()
array.push(rolled, bar_index)
array.push(retained, bar_index)
label.new(bar_index, close, "one per commit")
if bar_index == 0
    strategy.entry("A", strategy.long)
if strategy.opentrades == 1
    strategy.exit("TP", "A", limit=108)
log.info(str.tostring(bar_index))
plot(regular)
plot(persistent)
plot(previous)
plot(array.size(rolled))
plot(array.size(retained))
plot(counter.regular)
plot(counter.persistent)
plot(close[1])
"#,
        &bars(),
    );
    assert_eq!(out.plots[0], [1.0, 2.0, 3.0]);
    assert_eq!(out.plots[1], [1.0, 4.0, 5.0]);
    assert_eq!(out.plots[2][1..], [1.0, 4.0]);
    assert_eq!(out.plots[3], [1.0, 2.0, 3.0]);
    assert_eq!(out.plots[4], [1.0, 4.0, 5.0]);
    assert_eq!(out.plots[5], [1.0, 2.0, 3.0]);
    assert_eq!(out.plots[6], [1.0, 4.0, 5.0]);
    assert_eq!(out.plots[7][1..], [100.0, 105.0]);
    assert_eq!(run.drawings().labels.len(), 3);
    assert_eq!(run.logs().len(), 5);
}

#[test]
fn market_created_by_fill_waits_for_a_later_tick() {
    let (run, _) = run(
        r#"//@version=6
strategy("COOF successive market", calc_on_order_fills=true, pyramiding=2)
if bar_index == 0
    strategy.entry("A", strategy.long)
if strategy.opentrades == 1
    strategy.entry("B", strategy.long)
"#,
        &bars(),
    );
    let b = run.broker().unwrap();
    let fills: Vec<_> = b.fills.iter().map(|f| (f.id.as_str(), f.bar, f.price)).collect();
    assert_eq!(fills, [("A", 1, 100.0), ("B", 1, 110.0)]);
}

#[test]
fn immediately_closes_at_the_fill_tick_without_enabling_process_on_close() {
    let (run, out) = run(
        r#"//@version=6
strategy("COOF immediate", calc_on_order_fills=true)
varip int passes = 0
passes += 1
if bar_index == 0
    strategy.entry("A", strategy.long, limit=95)
if strategy.opentrades == 1
    strategy.close("A", immediately=true)
plot(passes)
"#,
        &bars(),
    );
    let b = run.broker().unwrap();
    assert_eq!(b.fills.len(), 2);
    assert_eq!((b.fills[0].bar, b.fills[0].price), (1, 95.0));
    assert_eq!((b.fills[1].bar, b.fills[1].price), (1, 95.0));
    assert_eq!(b.trades.len(), 1);
    assert_eq!(out.plots[0], [1.0, 4.0, 5.0]);
}

#[test]
fn a_buy_limit_does_not_take_adverse_slippage() {
    let (run, _) = run(
        r#"//@version=6
strategy("COOF limit price", calc_on_order_fills=true, slippage=3)
if bar_index == 0
    strategy.entry("A", strategy.long, limit=95)
"#,
        &bars(),
    );
    let b = run.broker().unwrap();
    assert_eq!(b.fills.len(), 1);
    assert_eq!(b.fills[0].price, 95.0);
}

#[test]
fn stop_limit_stays_armed_and_both_limits_require_verification() {
    let mut data = bars().to_vec();
    data[1].open = 105.0;
    data[1].low = 94.99;
    data[1].close = 102.0;
    data[2] = Bar {
        open: 102.0,
        high: 104.0,
        low: 94.90,
        close: 99.01,
        ..data[2]
    };
    data.push(Bar {
        open: 99.0,
        high: 100.0,
        low: 98.0,
        close: 100.0,
        ..data[2]
    });
    let (run, _) = run(
        r#"//@version=6
strategy("COOF stop-limit", calc_on_order_fills=true, backtest_fill_limits_assumption=5)
if strategy.position_size == 0 and strategy.closedtrades == 0
    strategy.entry("L", strategy.long, limit=95, stop=108)
if strategy.position_size > 0
    strategy.exit("TP", "L", limit=99)
"#,
        &data,
    );
    let b = run.broker().unwrap();
    let fills: Vec<_> = b.fills.iter().map(|f| (f.id.as_str(), f.bar, f.price)).collect();
    assert_eq!(fills, [("L", 2, 95.0), ("TP", 3, 99.0)]);
}

#[test]
fn verified_limits_at_one_tick_keep_the_limit_price() {
    let (run, out) = run(
        r#"//@version=6
strategy("COOF verified limits", calc_on_order_fills=true, backtest_fill_limits_assumption=5)
varip int passes = 0
passes += 1
if bar_index == 0
    strategy.order("A", strategy.long, limit=95)
    strategy.order("B", strategy.long, limit=95)
plot(passes)
"#,
        &bars(),
    );
    let fills: Vec<_> = run.broker().unwrap().fills.iter().map(|f| f.price).collect();
    assert_eq!(fills, [95.0, 95.0]);
    assert_eq!(out.plots[0], [1.0, 3.0, 4.0]);
}

#[test]
fn fill_limit_can_halt_a_reversal_before_its_opening_leg() {
    let (run, _) = run(
        r#"//@version=6
strategy("COOF risk", calc_on_order_fills=true)
strategy.risk.max_intraday_filled_orders(2)
if bar_index == 0
    strategy.entry("A", strategy.long)
if strategy.position_size > 0
    strategy.entry("R", strategy.short)
"#,
        &bars()[..2],
    );
    let b = run.broker().unwrap();
    let fills: Vec<_> = b.fills.iter().map(|f| (f.id.as_str(), f.exit)).collect();
    assert_eq!(fills, [("A", false), ("R", true)]);
    assert!(b.halted());
    assert!(b.pending.is_empty());
    assert!(b.lots.is_empty());
}

#[test]
fn forming_bar_recomputation_does_not_duplicate_committed_results() {
    use opentern_pine::PineStudy;
    use opentern_series::study::Instance;

    let script = compile(
        r#"//@version=6
strategy("COOF forming", calc_on_order_fills=true)
if bar_index == 0
    strategy.entry("A", strategy.long)
if strategy.opentrades == 1
    strategy.exit("TP", "A", limit=108)
"#,
        "order_fills.pine",
        Env::default(),
    )
    .unwrap();
    let mut instance = Instance::with_factory(script.meta(), &Inputs::default(), script.factory());
    let tf = Timeframe::new(1, Unit::Day);
    let bars = bars();
    for _ in 0..2 {
        instance.update(&bars[..2], &[], tf);
        let committed = instance
            .state()
            .unwrap()
            .downcast_ref::<PineStudy>()
            .unwrap()
            .broker()
            .unwrap();
        let forming = instance
            .latest_state()
            .unwrap()
            .downcast_ref::<PineStudy>()
            .unwrap()
            .broker()
            .unwrap();
        assert!(committed.fills.is_empty());
        assert_eq!(forming.fills.len(), 2);
        assert_eq!(forming.trades.len(), 1);
    }
    instance.update(&bars, &[], tf);
    let committed = instance
        .state()
        .unwrap()
        .downcast_ref::<PineStudy>()
        .unwrap()
        .broker()
        .unwrap();
    assert_eq!(committed.fills.len(), 2);
    assert_eq!(committed.trades.len(), 1);
    assert_eq!(committed.equity.len(), 2);
}

#[test]
fn reversal_reports_each_fill_once() {
    let (run, out) = run(
        r#"//@version=6
strategy("COOF reversal", calc_on_order_fills=true)
varip bool saw_flat = false
if bar_index == 0
    strategy.entry("A", strategy.long)
if strategy.position_size > 0
    strategy.entry("R", strategy.short)
if strategy.closedtrades == 1 and strategy.position_size == 0
    saw_flat := true
plot(saw_flat ? 1 : 0)
"#,
        &bars(),
    );
    let b = run.broker().unwrap();
    let fills: Vec<_> = b.fills.iter().map(|f| (f.id.as_str(), f.exit, f.price)).collect();
    assert_eq!(fills, [("A", false, 100.0), ("R", true, 110.0), ("R", false, 110.0)]);
    assert_eq!(out.plots[0], [0.0, 1.0, 1.0]);
    assert_eq!(b.trades.len(), 1);
    assert_eq!(b.position, [0.0, -1.0, -1.0]);
}

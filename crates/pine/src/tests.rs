//! Language semantics, end to end: source in, values per bar out.

use crate::{Env, compile};
use opentern_core::{Bar, Timeframe, Unit};
use opentern_series::study::{Inputs, Instance, Value};

fn bars(n: usize) -> Vec<Bar> {
    (0..n)
        .map(|i| {
            let x = i as f64;
            let c = 100.0 + (x * 0.7).sin() * 3.0 + x * 0.1;
            Bar {
                time: 1_600_000_000_000 + i as i64 * 3_600_000,
                open: c - 0.5,
                high: c + 1.0,
                low: c - 1.0,
                close: c,
                volume: 1000.0 + x,
            }
        })
        .collect()
}

const H1: Timeframe = Timeframe::new(1, Unit::Hour);

/// Runs `body` (after an `indicator` line) and returns every plot.
fn run(body: &str, n: usize) -> Vec<Vec<f64>> {
    let src = format!("//@version=6\nindicator(\"T\")\n{body}");
    let s = compile(&src, "t.pine", Env::default()).unwrap_or_else(|e| panic!("{e}\n{src}"));
    let mut st = s.study(&Inputs::default());
    let out = st.run(&bars(n), H1);
    if let Some(e) = st.error() {
        panic!("runtime: {e}");
    }
    out.plots
}

fn plot(body: &str, n: usize) -> Vec<f64> {
    run(body, n).remove(0)
}

fn err(body: &str) -> crate::Error {
    let src = format!("//@version=6\nindicator(\"T\")\n{body}");
    match compile(&src, "t.pine", Env::default()) {
        Err(e) => e,
        Ok(s) => {
            let mut st = s.study(&Inputs::default());
            st.run(&bars(5), H1);
            st.error().cloned().expect("an error")
        }
    }
}

#[test]
fn var_persists_and_plain_vars_restart() {
    assert_eq!(plot("var c = 0\nc += 1\nplot(c)", 4), vec![1.0, 2.0, 3.0, 4.0]);
    assert_eq!(plot("c = 0\nc += 1\nplot(c)", 3), vec![1.0, 1.0, 1.0]);
    assert_eq!(plot("plot(bar_index)", 3), vec![0.0, 1.0, 2.0]);
}

#[test]
fn history_of_variables_and_expressions() {
    let v = plot("x = bar_index * 2\nplot(x[1])", 4);
    assert!(v[0].is_nan());
    assert_eq!(&v[1..], &[0.0, 2.0, 4.0]);
    let v = plot("plot((bar_index + 1)[2])", 4);
    assert_eq!(v[2], 1.0);
    let v = plot("plot(close[1] - close[1])", 3);
    assert!(v[0].is_nan() && v[1] == 0.0);
    // Self-referencing history through :=
    let v = plot("s = 0.0\ns := nz(s[1]) + 1\nplot(s)", 4);
    assert_eq!(v, vec![1.0, 2.0, 3.0, 4.0]);
    // Dynamic offsets
    let v = plot("k = bar_index % 2\nplot(bar_index[k])", 4);
    assert_eq!(v, vec![0.0, 0.0, 2.0, 2.0]);
}

#[test]
fn each_call_site_has_its_own_state() {
    let p = run(
        "f(x) =>\n    var n = 0\n    n += 1\n    ta.sma(x, 2) + n * 0\ng() =>\n    var n = 0\n    n += 1\n    n\n\
         plot(f(close))\nplot(f(open))\nplot(g())\nplot(g() + g())",
        4,
    );
    let b = bars(4);
    assert!(p[0][0].is_nan());
    assert!((p[0][1] - (b[0].close + b[1].close) / 2.0).abs() < 1e-12);
    assert!((p[1][1] - (b[0].open + b[1].open) / 2.0).abs() < 1e-12);
    assert_eq!(p[2], vec![1.0, 2.0, 3.0, 4.0]);
    // Two call sites, two counters.
    assert_eq!(p[3], vec![2.0, 4.0, 6.0, 8.0]);
}

#[test]
fn function_locals_have_history_only_when_called() {
    // f runs on even bars only; x[1] inside it is the previous call's value.
    let v = plot(
        "f() =>\n    x = bar_index\n    x[1]\nr = bar_index % 2 == 0 ? f() : na\nplot(r)",
        7,
    );
    assert!(v[0].is_nan() && v[1].is_nan());
    assert_eq!(v[2], 0.0);
    assert_eq!(v[4], 2.0);
    assert_eq!(v[6], 4.0);
}

#[test]
fn control_flow() {
    let v = plot(
        "s = 0\nfor i = 0 to 10\n    if i == 3\n        continue\n    if i > 5\n        break\n    s += i\nplot(s)",
        1,
    );
    assert_eq!(v[0], 12.0);
    assert_eq!(plot("s = 0\nfor i = 3 to 1\n    s := s * 10 + i\nplot(s)", 1)[0], 321.0);
    assert_eq!(plot("s = 0.0\nfor i = 0 to 1 by 0.25\n    s += i\nplot(s)", 1)[0], 2.5);
    assert_eq!(plot("n = 0\nwhile n < 7\n    n += 2\nplot(n)", 1)[0], 8.0);
    let v = plot(
        "x = if bar_index == 0\n    10\nelse if bar_index == 1\n    20\nelse\n    30\nplot(x)",
        3,
    );
    assert_eq!(v, vec![10.0, 20.0, 30.0]);
    let v = plot(
        "y = switch bar_index\n    0 => 5\n    1 => 6\n    => 7\nz = switch\n    bar_index > 1 => 1\n    => 0\nplot(y * 10 + z)",
        3,
    );
    assert_eq!(v, vec![50.0, 60.0, 71.0]);
    let v = plot("x = if false\n    1\nplot(na(x) ? -1 : x)", 1);
    assert_eq!(v[0], -1.0);
}

#[test]
fn laziness_follows_the_version() {
    // v6: `and` doesn't evaluate its right side when the left is false, so
    // the counter in f() only moves on odd bars.
    let body = "var c = 0\nf() =>\n    c2 = 1\n    c2\nn = 0\ncount() =>\n    var k = 0\n    k += 1\n    k\n\
                b = bar_index % 2 == 1 and count() > 0\nplot(count())";
    let v = plot(body, 4);
    // The second call site counts every bar.
    assert_eq!(v, vec![1.0, 2.0, 3.0, 4.0]);
    let lazy = plot(
        "count() =>\n    var k = 0\n    k += 1\n    k\nx = bar_index % 2 == 1 and count() > 0 ? 1 : 0\nv = 0\nplot(ta.cum(x))",
        4,
    );
    assert_eq!(lazy, vec![0.0, 1.0, 1.0, 2.0]);
    // Ternaries only evaluate the chosen side: the sma only sees odd bars.
    let v = plot("plot(bar_index % 2 == 1 ? ta.sma(bar_index, 2) : na)", 6);
    assert!(v[1].is_nan());
    assert_eq!(v[3], 2.0);
    assert_eq!(v[5], 4.0);
}

#[test]
fn tuples_and_functions() {
    let p = run("f(a) =>\n    [a, a * 2]\n[x, y] = f(bar_index)\nplot(x)\nplot(y)", 3);
    assert_eq!(p[1], vec![0.0, 2.0, 4.0]);
    let p = run("[m, s, h] = ta.macd(close, 3, 5, 2)\nplot(h)\nplot(m - s)", 30);
    for (a, b) in p[0].iter().zip(&p[1]) {
        assert!(a.is_nan() && b.is_nan() || (a - b).abs() < 1e-12);
    }
    assert_eq!(plot("f(x, k = 3) => x * k\nplot(f(2) + f(2, k = 1))", 1)[0], 8.0);
}

#[test]
fn values_and_na() {
    assert_eq!(plot("plot(nz(na, 5) + nz(close[1]) * 0)", 1)[0], 5.0);
    assert_eq!(plot("plot(na(close[1]) ? 1 : 0)", 2), vec![1.0, 0.0]);
    assert_eq!(plot("plot(7 / 2)", 1)[0], 3.5);
    assert_eq!(plot("plot(7 % 3)", 1)[0], 1.0);
    assert!(plot("plot(1 / 0)", 1)[0].is_nan());
    assert!(plot("plot(close / (close - close))", 1)[0].is_nan());
    assert_eq!(plot("plot(math.max(1, 5, 3) + math.min(2, -1))", 1)[0], 4.0);
    assert_eq!(plot("plot(math.round(2.5) + math.round(1.234, 2))", 1)[0], 3.0 + 1.23);
    assert_eq!(plot("plot(int(3.9) + int(-3.9))", 1)[0], 0.0);
    assert_eq!(plot("plot(color.r(color.new(#102030, 50)))", 1)[0], 16.0);
    assert_eq!(plot("plot(color.t(color.new(color.red, 70)))", 1)[0], 70.0);
}

#[test]
fn strings() {
    assert_eq!(plot("plot(str.length(\"abc\" + str.tostring(12)))", 1)[0], 5.0);
    assert_eq!(plot("plot(str.tonumber(\"1.5\") * 2)", 1)[0], 3.0);
    assert_eq!(
        plot("plot(str.contains(str.upper(\"pine\"), \"IN\") ? 1 : 0)", 1)[0],
        1.0
    );
    assert_eq!(plot("plot(str.pos(\"hello\", \"l\"))", 1)[0], 2.0);
    assert_eq!(
        plot(
            "s = str.format(\"{0}-{1,number,#.0}\", \"a\", 2.26)\nplot(s == \"a-2.3\" ? 1 : 0)",
            1
        )[0],
        1.0
    );
    assert_eq!(
        plot("plot(str.substring(\"abcdef\", 1, 3) == \"bc\" ? 1 : 0)", 1)[0],
        1.0
    );
}

#[test]
fn time_and_timeframe() {
    // 2020-09-13 12:26:40 UTC is the first bar.
    let p = run(
        "plot(year)\nplot(month)\nplot(dayofmonth)\nplot(hour)\nplot(dayofweek)",
        1,
    );
    assert_eq!(
        (p[0][0], p[1][0], p[2][0], p[3][0], p[4][0]),
        (2020.0, 9.0, 13.0, 12.0, 1.0)
    );
    assert_eq!(
        plot("plot(timeframe.isintraday ? timeframe.multiplier : -1)", 1)[0],
        60.0
    );
    assert_eq!(
        plot("plot(timestamp(2020, 1, 2, 0, 0) - timestamp(2020, 1, 1, 0, 0))", 1)[0],
        86_400_000.0
    );
}

#[test]
fn inputs_become_settings() {
    let src = "//@version=5\nindicator(\"Inp\", overlay = true)\nlen = input.int(14, \"Length\", minval = 1, maxval = 50)\n\
               src = input.source(close, \"Source\")\nmode = input.string(\"A\", \"Mode\", options = [\"A\", \"B\"])\n\
               on = input.bool(true, \"Show\")\nc = input.color(#FF0000, \"Colour\")\nplot(on and mode == \"B\" ? ta.sma(src, len) : src, color = c)";
    let s = compile(src, "inp.pine", Env::default()).unwrap();
    let m = s.meta();
    assert_eq!(m.id, "pine:inp.pine");
    assert_eq!(m.name, "Inp");
    let ids: Vec<&str> = m.inputs.iter().map(|i| i.id).collect();
    assert_eq!(ids, vec!["length", "source", "mode", "show", "colour"]);
    assert!(matches!(
        m.inputs[0].kind,
        opentern_series::study::InputKind::Int { min: 1, max: 50 }
    ));
    assert_eq!(m.plots[0].colors[0].1.0, 0xFF0000FF);
    let b = bars(30);
    let mut inputs = m.default_inputs();
    let base = s.study(&inputs).run(&b, H1);
    assert_eq!(base.plots[0][20], b[20].close);
    inputs.set("mode", Value::Text("B".into()));
    inputs.set("length", Value::Int(3));
    inputs.set("source", Value::Text("open".into()));
    let changed = s.study(&inputs.normalized(m)).run(&b, H1);
    let want = (b[18].open + b[19].open + b[20].open) / 3.0;
    assert!((changed.plots[0][20] - want).abs() < 1e-12);
}

#[test]
fn plot_colours_map_to_the_palette() {
    let src = "//@version=5\nindicator(\"C\")\nplot(close, color = close > close[1] ? color.green : color.red)\n\
               plot(close, color = color.rgb(bar_index % 256, 0, 0))\nbgcolor(bar_index % 2 == 0 ? color.new(color.blue, 80) : na)";
    let s = compile(src, "c.pine", Env::default()).unwrap();
    assert_eq!(s.meta().plots[0].colors.len(), 2);
    let out = s.study(&Inputs::default()).run(&bars(6), H1);
    // Palette indices for constant colours, raw colours otherwise.
    assert!(out.colors[0].iter().all(|c| *c <= 1));
    assert!(out.raw[0].iter().all(|c| *c == 0));
    assert_eq!(out.raw[1][3], 0x030000FF);
    assert_ne!(out.bg[0], 0);
    assert_eq!(out.bg[1], 0);
}

#[test]
fn errors_name_the_line() {
    let e = err("x = 1\ny = foo + 1");
    assert_eq!(e.line, 4);
    assert!(e.message.contains("foo"), "{}", e.message);
    let e = err("if close > open\n    plot(close)");
    assert!(e.message.contains("local"), "{}", e.message);
    let e = err("x = 1\nruntime.error(\"boom\")");
    assert_eq!((e.line, e.message.as_str()), (4, "boom"));
    let e = err("n = 0\nwhile true\n    n += 1");
    assert!(e.message.contains("loop"), "{}", e.message);
    let e = err("plot(ta.sma(close, 0))");
    assert!(e.message.contains("greater than 0"), "{}", e.message);
    assert!(compile("plot(close)", "x", Env::default()).is_err());
}

#[test]
fn forming_bar_rolls_back() {
    // Through the real runtime: closed bars are stepped once, the last bar
    // on a clone each time it changes.
    let src = "//@version=5\nindicator(\"Count\")\nvar n = 0\nn += 1\nplot(n)\nplot(ta.sma(close, 3))";
    let s = compile(src, "count.pine", Env::default()).unwrap();
    let meta = s.meta();
    let mut inst = Instance::with_factory(meta, &meta.default_inputs(), s.factory());
    let mut b = bars(5);
    let sess = vec![false; 5];
    inst.recompute(&b, &sess, H1);
    assert_eq!(inst.out.plots[0], vec![1.0, 2.0, 3.0, 4.0, 5.0]);
    // Ticks on the forming bar don't add up.
    for k in 0..3 {
        b[4].close += 1.0 + k as f64;
        inst.update(&b, &sess, H1);
        assert_eq!(inst.out.plots[0][4], 5.0);
        let want = (b[2].close + b[3].close + b[4].close) / 3.0;
        assert!((inst.out.plots[1][4] - want).abs() < 1e-12);
    }
    // A new bar commits the old one.
    let mut next = b[4];
    next.time += 3_600_000;
    b.push(next);
    let sess = vec![false; 6];
    inst.update(&b, &sess, H1);
    assert_eq!(inst.out.plots[0], vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
}

#[test]
fn shapes_and_candles() {
    let src = "//@version=5\nindicator(\"S\", overlay = true)\nplotshape(close > open, style = shape.triangleup, location = location.belowbar)\n\
               plotchar(bar_index == 1, char = \"x\")\nplotcandle(open, high, low, close, color = close > open ? color.green : color.red)";
    let s = compile(src, "s.pine", Env::default()).unwrap();
    let m = s.meta();
    assert_eq!(m.plots.len(), 2 + 4);
    assert_eq!(m.plots[2].style, opentern_series::study::PlotStyle::Candle);
    let b = bars(3);
    let out = s.study(&Inputs::default()).run(&b, H1);
    assert_eq!(out.plots[0][0], 1.0);
    assert!(out.plots[1][0].is_nan() && out.plots[1][1] == 1.0);
    assert_eq!(out.plots[5][2], b[2].close);
}

#[test]
fn comma_declarations_and_continuations() {
    let v = plot("a = 1, b = 2\nc = a +\n  b\nplot(c)", 1);
    assert_eq!(v[0], 3.0);
}

/// A Pine strategy and the same logic driven straight at the broker give the
/// same trades.
#[test]
fn strategy_orders_go_to_the_broker() {
    use opentern_engine::{Broker, Order, OrderKind, Props};
    let src = "//@version=5\nstrategy(\"Cross\", overlay = true, initial_capital = 10000, default_qty_type = strategy.percent_of_equity, default_qty_value = 50, commission_type = strategy.commission.percent, commission_value = 0.1)\n\
               f = ta.sma(close, 3)\ns = ta.sma(close, 8)\nif ta.crossover(f, s)\n    strategy.entry(\"L\", strategy.long)\n\
               if ta.crossunder(f, s)\n    strategy.entry(\"S\", strategy.short)\nplot(strategy.position_size)";
    let s = compile(src, "x.pine", Env::default()).unwrap();
    assert_eq!(s.kind(), crate::Kind::Strategy);
    let b = bars(300);
    let mut st = s.study(&Inputs::default());
    let out = st.run(&b, H1);
    assert!(st.error().is_none(), "{:?}", st.error());
    let pine = st.broker().unwrap().clone();
    assert!(pine.trades.len() > 5, "{} trades", pine.trades.len());
    // The same by hand.
    let props = Props::from_json(s.strategy_props().unwrap(), 0.01);
    assert_eq!(props.initial_capital, 10000.0);
    assert_eq!(props.qty_type, opentern_engine::QtyType::PercentOfEquity);
    let mut br = Broker::new(props);
    let (mut f, mut sl) = (opentern_series::ta::Sma::new(3), opentern_series::ta::Sma::new(8));
    let mut cross = opentern_series::ta::Cross::default();
    for (i, x) in b.iter().enumerate() {
        br.open_bar(i, x);
        let (over, under) = cross.update(f.update(x.close), sl.update(x.close));
        if over {
            let mut o = Order::market("L", OrderKind::Entry, true);
            o.comment = "L".into();
            br.submit(o);
        }
        if under {
            let mut o = Order::market("S", OrderKind::Entry, false);
            o.comment = "S".into();
            br.submit(o);
        }
        br.close_bar(x);
    }
    assert_eq!(pine.trades, br.trades);
    assert_eq!(pine.equity, br.equity);
    // strategy.position_size is read on each bar's close.
    assert_eq!(out.plots[0], br.position);
}

#[test]
fn strategy_exits_and_variables() {
    let src = "//@version=5\nstrategy(\"Bracket\", process_orders_on_close = false)\n\
               if bar_index == 2\n    strategy.entry(\"L\", strategy.long, qty = 2)\n\
               strategy.exit(\"X\", \"L\", profit = 400, loss = 400)\n\
               plot(strategy.closedtrades)\nplot(strategy.opentrades)\nplot(strategy.closedtrades > 0 ? strategy.closedtrades.exit_bar_index(0) : na)";
    let s = compile(src, "b.pine", Env::default()).unwrap();
    let b = bars(60);
    let mut st = s.study(&Inputs::default());
    let out = st.run(&b, H1);
    assert!(st.error().is_none(), "{:?}", st.error());
    let br = st.broker().unwrap();
    assert_eq!(br.trades.len(), 1);
    let t = &br.trades[0];
    assert_eq!(t.entry_bar, 3);
    assert_eq!(t.qty, 2.0);
    // Exited at least 4 away from the entry price (400 ticks of 0.01;
    // more when the bar gapped through the level and filled at its open).
    let moved = (t.exit_price - t.entry_price).abs();
    let at_open = t.exit_price == b[t.exit_bar].open;
    assert!((moved - 4.0).abs() < 1e-9 || (at_open && moved > 4.0), "{t:?}");
    assert!(t.exit_bar > 3);
    assert_eq!(out.plots[1][3], 1.0);
    assert_eq!(out.plots[0][t.exit_bar], 1.0);
    assert_eq!(out.plots[2][t.exit_bar], t.exit_bar as f64);
    // strategy.* in an indicator is an error, with the line.
    let e = err("strategy.entry(\"L\", strategy.long)");
    assert!(e.message.contains("strategy("), "{}", e.message);
}

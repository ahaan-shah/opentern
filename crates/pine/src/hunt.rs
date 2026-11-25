//! Bugs found by reading the engine against the Pine manual, each with the
//! script that showed it.

use crate::{Env, Script, compile};
use opentern_core::{Bar, Timeframe, Unit};
use opentern_series::study::{Inputs, Study};

const H1: Timeframe = Timeframe::new(1, Unit::Hour);
const M15: Timeframe = Timeframe::new(15, Unit::Minute);
const D1: Timeframe = Timeframe::new(1, Unit::Day);

fn ms(s: &str) -> i64 {
    s.parse::<jiff::Timestamp>().unwrap().as_millisecond()
}

fn bar(time: i64, x: f64) -> Bar {
    Bar {
        time,
        open: x,
        high: x + 1.0,
        low: x - 1.0,
        close: x,
        volume: 10.0,
    }
}

/// Wavy hourly bars from a UTC midnight (crypto: every hour).
fn hours(n: usize) -> Vec<Bar> {
    let t0 = ms("2024-01-01T00:00:00Z");
    (0..n)
        .map(|i| {
            let x = i as f64;
            bar(t0 + i as i64 * 3_600_000, 100.0 + (x * 0.7).sin() * 3.0 + x * 0.1)
        })
        .collect()
}

/// Weekdays stamped at 00:00 UTC, as the app's daily bars are.
fn weekdays(from: &str, n: usize) -> Vec<Bar> {
    let mut t = ms(from);
    let mut out = Vec::new();
    while out.len() < n {
        if (t.div_euclid(86_400_000) + 3).rem_euclid(7) < 5 {
            out.push(bar(t, 100.0 + out.len() as f64));
        }
        t += 86_400_000;
    }
    out
}

fn us() -> Env {
    Env {
        tickerid: "NASDAQ:AAPL".into(),
        kind: "stock".into(),
        timezone: "America/New_York".into(),
        ..Env::default()
    }
}

fn india() -> Env {
    Env {
        tickerid: "NSE:RELIANCE".into(),
        kind: "stock".into(),
        currency: "INR".into(),
        mintick: 0.05,
        timezone: "Asia/Kolkata".into(),
        ..Env::default()
    }
}

fn load(v: u8, body: &str, env: Env) -> Script {
    let src = format!("//@version={v}\nindicator(\"T\", overlay = true)\n{body}");
    compile(&src, "t.pine", env).unwrap_or_else(|e| panic!("compile: {e}\n{src}"))
}

fn on(v: u8, body: &str, b: &[Bar], tf: Timeframe, env: Env) -> Vec<Vec<f64>> {
    let mut st = load(v, body, env).study(&Inputs::default());
    let out = st.run(b, tf);
    if let Some(e) = st.error() {
        panic!("runtime: {e}\n{body}");
    }
    out.plots
}

fn run(body: &str, n: usize) -> Vec<Vec<f64>> {
    on(6, body, &hours(n), H1, Env::default())
}

fn plot(body: &str, n: usize) -> Vec<f64> {
    run(body, n).remove(0)
}

fn last(body: &str) -> f64 {
    *plot(body, 40).last().unwrap()
}

/// The last label's text: how the tests see strings.
fn text(v: u8, body: &str) -> String {
    let mut st = load(
        v,
        &format!("{body}\nif barstate.islast\n    label.new(bar_index, close, out)"),
        Env::default(),
    )
    .study(&Inputs::default());
    st.run(&hours(40), H1);
    if let Some(e) = st.error() {
        panic!("runtime: {e}\n{body}");
    }
    st.drawings().labels.last().expect("a label").text.to_string()
}

fn fed(env: &Env, sym: &str, tf: Timeframe, bars: Vec<Bar>) {
    env.feeds
        .write()
        .unwrap()
        .insert(crate::feed_key(sym, tf), std::sync::Arc::new(bars));
}

fn eq(a: f64, b: f64) -> bool {
    (a.is_nan() && b.is_nan()) || (a - b).abs() < 1e-9 * (1.0 + b.abs())
}

#[track_caller]
fn same(a: &[f64], b: &[f64]) {
    assert_eq!(a.len(), b.len());
    for (i, (x, y)) in a.iter().zip(b).enumerate() {
        assert!(eq(*x, *y), "bar {i}: {x} vs {y}\n{a:?}\n{b:?}");
    }
}

// ------------------------------------------------------------ request.security

#[test]
fn sec_function_reads_an_input() {
    let p = run(
        "len = input.int(5)\nf() => ta.sma(close, len)\nplot(request.security(syminfo.tickerid, \"240\", f()))\n\
         plot(request.security(syminfo.tickerid, \"240\", ta.sma(close, 5)))",
        120,
    );
    assert!(p[1][119].is_finite());
    same(&p[0], &p[1]);
}

#[test]
fn sec_function_reads_a_global_series_from_the_requested_symbol() {
    let env = Env::default();
    let b = hours(30);
    let other: Vec<Bar> = b
        .iter()
        .enumerate()
        .map(|(i, x)| bar(x.time, 1000.0 + i as f64))
        .collect();
    fed(&env, "BINANCE:AAAUSDT", H1, other.clone());
    let p = on(
        6,
        "src = close * 2\nf() => src + src[1]\nplot(request.security(\"BINANCE:AAAUSDT\", \"60\", f()))",
        &b,
        H1,
        env,
    );
    assert_eq!(p[0][10], other[10].close * 2.0 + other[9].close * 2.0);
}

#[test]
fn sec_in_a_function_takes_its_parameters() {
    let p = run(
        "f(sym, tf, src) => request.security(sym, tf, src)\nplot(f(syminfo.tickerid, \"240\", ta.ema(close, 3)))\n\
         plot(request.security(syminfo.tickerid, \"240\", ta.ema(close, 3)))\n\
         g(src, n) => request.security(syminfo.tickerid, \"240\", src[n])\nplot(g(close, 1))\n\
         plot(request.security(syminfo.tickerid, \"240\", close[1]))\n\
         h(s) => request.security(syminfo.tickerid, \"240\", s)\nk(s) => h(s * 2)\nplot(k(close))\n\
         plot(request.security(syminfo.tickerid, \"240\", close * 2))",
        120,
    );
    same(&p[0], &p[1]);
    same(&p[2], &p[3]);
    same(&p[4], &p[5]);
}

#[test]
fn sec_in_a_function_takes_its_locals() {
    let p = run(
        "f(n) =>\n    m = ta.sma(close, n)\n    d = close - m\n    request.security(syminfo.tickerid, \"240\", d)\nplot(f(3))\n\
         plot(request.security(syminfo.tickerid, \"240\", close - ta.sma(close, 3)))",
        120,
    );
    same(&p[0], &p[1]);
}

#[test]
fn sec_takes_tuple_members_and_blocks() {
    let p = run(
        "[m, s, h] = ta.macd(close, 3, 6, 2)\nplot(request.security(syminfo.tickerid, \"240\", h))\n\
         plot(request.security(syminfo.tickerid, \"240\", (ta.ema(close, 3) - ta.ema(close, 6)) - ta.ema(ta.ema(close, 3) - ta.ema(close, 6), 2)))\n\
         x = if close > open\n    1\nelse\n    2\nplot(request.security(syminfo.tickerid, \"240\", x))",
        200,
    );
    same(&p[0], &p[1]);
}

#[test]
fn sec_over_a_list_of_symbols() {
    let env = Env::default();
    let b = hours(40);
    let mk = |k: f64| -> Vec<Bar> { b.iter().enumerate().map(|(i, x)| bar(x.time, k + i as f64)).collect() };
    let (a, z) = (mk(10.0), mk(500.0));
    fed(&env, "BINANCE:AAAUSDT", H1, a.clone());
    fed(&env, "BINANCE:ZZZUSDT", H1, z.clone());
    let p = on(
        6,
        "syms = array.from(\"BINANCE:AAAUSDT\", \"BINANCE:ZZZUSDT\")\nt = 0.0\nfor s in syms\n    t += request.security(s, \"60\", ta.sma(close, 3))\nplot(t)",
        &b,
        H1,
        env,
    );
    assert!(eq(p[0][20], a[19].close + z[19].close), "{}", p[0][20]);
}

#[test]
fn sec_over_a_list_of_timeframes() {
    let p = run(
        "t = 0.0\nfor tf in array.from(\"120\", \"240\")\n    t += request.security(syminfo.tickerid, tf, ta.sma(close, 3))\nplot(t)\n\
         plot(request.security(syminfo.tickerid, \"120\", ta.sma(close, 3)) + request.security(syminfo.tickerid, \"240\", ta.sma(close, 3)))",
        120,
    );
    same(&p[0], &p[1]);
}

#[test]
fn sec_called_twice_in_one_bar_counts_the_bar_once() {
    let p = run(
        "t = 0.0\nfor i = 1 to 2\n    t := request.security(syminfo.tickerid, \"240\", volume)\nplot(t)\n\
         plot(request.security(syminfo.tickerid, \"240\", volume))\n\
         u = 0.0\nfor i = 1 to 2\n    u := request.security(syminfo.tickerid, \"60\", ta.cum(1))\nplot(u)",
        60,
    );
    same(&p[0], &p[1]);
    assert_eq!(p[2][9], 10.0);
}

#[test]
fn sec_in_a_branch_still_sees_every_bar() {
    let p = run(
        "x = bar_index % 4 == 3 ? request.security(syminfo.tickerid, \"240\", high) : na\nplot(x)\n\
         y = request.security(syminfo.tickerid, \"240\", high)\nplot(bar_index % 4 == 3 ? y : na)\n\
         z = bar_index % 2 == 1 ? request.security(syminfo.tickerid, \"60\", close[1]) : na\nplot(z)\nplot(bar_index % 2 == 1 ? close[1] : na)",
        60,
    );
    same(&p[0], &p[1]);
    same(&p[2], &p[3]);
}

#[test]
fn sec_takes_state_built_with_var() {
    // A scanner's condition: a state machine, a counter, an array, a function
    // that reads them. On the chart's own timeframe it's the same by hand.
    let body = "n = input.int(3)\nvar int st = 0\nvar float top = na\nvar hist = array.new<float>()\nup = close > open\nif up\n    st += 1\n    top := na(top) ? high : math.max(top, high)\nelse\n    st := 0\nif barstate.isconfirmed\n    hist.push(close)\n    if hist.size() > n\n        hist.shift()\navg() => hist.size() > 0 ? hist.avg() : na\nl = label.new(bar_index, st)\nx = 0.0\nx := nz(x[1]) + (up ? 1 : -1)\ncond = st >= 2 and avg() < close\n";
    let b = walk(&hours(300));
    let p = on(
        6,
        &format!(
            "{body}plot(request.security(syminfo.tickerid, \"60\", cond ? st + top + x + avg() : -x))\nplot(cond ? st + top + x + avg() : -x)"
        ),
        &b,
        H1,
        Env::default(),
    );
    same(&p[0], &p[1]);
    // On another symbol: what that symbol's own chart says.
    let env = Env::default();
    let other = walk(&walk(&hours(300)));
    fed(&env, "BINANCE:AAAUSDT", H1, other.clone());
    let q = on(
        6,
        &format!(
            "{body}f(s) => request.security(s, \"60\", cond ? st + top + x + avg() : -x)\nplot(f(\"BINANCE:AAAUSDT\"))"
        ),
        &b,
        H1,
        env,
    );
    let own = on(
        6,
        &format!("{body}plot(cond ? st + top + x + avg() : -x)"),
        &other,
        H1,
        Env::default(),
    );
    same(&q[0], &own[0]);
    // 4h: against 4h bars made by hand.
    let h4 = on(
        6,
        &format!("{body}plot(st * 1000 + x)"),
        &fours(&b),
        Timeframe::new(4, Unit::Hour),
        Env::default(),
    );
    let r = on(
        6,
        &format!("{body}plot(request.security(syminfo.tickerid, \"240\", st * 1000 + x))"),
        &b,
        H1,
        Env::default(),
    );
    for k in 0..75 {
        assert!(eq(r[0][k * 4 + 3], h4[0][k]), "4h bar {k}");
    }
}

#[test]
fn a_screener_collecting_into_a_matrix() {
    let env = Env::default();
    let b = walk(&hours(200));
    let other = walk(&b);
    fed(&env, "BINANCE:AAAUSDT", H1, other.clone());
    fed(&env, "BINANCE:BBBUSDT", H1, b.clone());
    let body = "var int run = 0\nbool up = false\nup := close > open\nrun := up ? run + 1 : 0\nsig = run >= 2\ncalc() => [time, close, sig ? run : na]\nvar m = matrix.new<float>()\nadd(a, b) => m.add_row(0, array.from(a, b))\nscreen(s) =>\n    sym = ticker.modify(s, syminfo.session)\n    [t, p, g] = request.security(sym, timeframe.period, calc())\n    if not na(g) and t == time\n        add(p, g)\nscreen(\"BINANCE:AAAUSDT\"), screen(\"BINANCE:BBBUSDT\")\nplot(m.rows() > 0 ? m.get(0, 1) : na)\nplot(m.rows())\n";
    let p = on(6, body, &b, H1, env);
    let runs = |b: &[Bar]| {
        let mut n = 0;
        b.iter()
            .filter(|x| {
                n = if x.close > x.open { n + 1 } else { 0 };
                n >= 2
            })
            .count() as f64
    };
    assert_eq!(p[1][199], runs(&b) + runs(&other));
}

#[test]
fn sec_state_ignores_local_names() {
    // `x` further down is a function's own and a block's own.
    let p = run(
        "var x = 0\nx += 1\nd = request.security(syminfo.tickerid, \"60\", x)\nf() =>\n    x = 5\n    x\nif close > 0\n    x = f()\n    label.new(bar_index, x)\nplot(d)",
        20,
    );
    assert_eq!(p[0][19], 20.0);
}

#[test]
fn requests_draw_and_order_nothing() {
    let mut st = load(6, "f() =>\n    label.new(bar_index, close, \"x\")\n    log.info(\"y\")\n    close\nplot(request.security(syminfo.tickerid, \"240\", f()))", Env::default()).study(&Inputs::default());
    st.run(&hours(40), H1);
    assert!(st.drawings().labels.is_empty() && st.logs().is_empty());
}

fn fours(b: &[Bar]) -> Vec<Bar> {
    b.chunks(4)
        .map(|c| Bar {
            time: c[0].time,
            open: c[0].open,
            high: c.iter().map(|x| x.high).fold(f64::MIN, f64::max),
            low: c.iter().map(|x| x.low).fold(f64::MAX, f64::min),
            close: c[c.len() - 1].close,
            volume: c.iter().map(|x| x.volume).sum(),
        })
        .collect()
}

#[test]
fn sec_state_changed_below_the_request() {
    // The request sees each variable as it stands at that line.
    let body = "x = close\nvar int n = 0\nvar float top = 0\nREQ\nx := x * 2\nn += 1\ng(v) => math.max(v, top)\nif close > open\n    top := g(high)\nz = n + top\n";
    let b = walk(&hours(200));
    let p = on(
        6,
        &format!(
            "{}plot(y)",
            body.replace(
                "REQ",
                "y = request.security(syminfo.tickerid, \"60\", x + n * 1000 + top)"
            )
        ),
        &b,
        H1,
        Env::default(),
    );
    let want = on(
        6,
        &format!("{}plot(y)", body.replace("REQ", "y = x + n * 1000 + top")),
        &b,
        H1,
        Env::default(),
    );
    same(&p[0], &want[0]);
    let p = on(
        6,
        &format!(
            "f(s) => request.security(s, \"240\", x + n * 1000 + top)\n{}plot(y)",
            body.replace("REQ", "y = f(syminfo.tickerid)")
        )
        .replace(
            "f(s) => request.security(s, \"240\", x + n * 1000 + top)\nx = close\nvar int n = 0\nvar float top = 0\n",
            "x = close\nvar int n = 0\nvar float top = 0\nf(s) => request.security(s, \"240\", x + n * 1000 + top)\n",
        ),
        &b,
        H1,
        Env::default(),
    );
    let want = on(
        6,
        &format!("{}plot(y)", body.replace("REQ", "y = x + n * 1000 + top")),
        &fours(&b),
        Timeframe::new(4, Unit::Hour),
        Env::default(),
    );
    for k in 0..50 {
        assert!(
            eq(p[0][k * 4 + 3], want[0][k]),
            "4h bar {k}: {} vs {}",
            p[0][k * 4 + 3],
            want[0][k]
        );
    }
    // A function sees the state where it's called, not where it's written.
    let q = on(
        6,
        "var n = 0\nf() => request.security(syminfo.tickerid, \"60\", n)\na = f()\nn += 1\nb = f()\nplot(a)\nplot(b)",
        &b,
        H1,
        Env::default(),
    );
    assert_eq!((q[0][10], q[1][10]), (10.0, 11.0));
}

#[test]
fn date_inputs_are_dates() {
    let s = load(
        6,
        "t = input.time(timestamp(\"2024-01-31T14:30:00Z\"), \"From\")\nplot(t)",
        us(),
    );
    assert_eq!(
        s.meta().inputs[0].default,
        opentern_series::study::Value::Text("2024-01-31 09:30".into())
    );
    let at = |i: &Inputs| s.study(i).run(&hours(1), H1).plots[0][0] as i64;
    assert_eq!(at(&Inputs::default()), ms("2024-01-31T14:30:00Z"));
    let set = |t: &str| {
        let mut i = Inputs::default();
        i.values
            .insert("from".into(), opentern_series::study::Value::Text(t.into()));
        i
    };
    assert_eq!(at(&set("2024-06-03")), ms("2024-06-03T04:00:00Z"));
    assert_eq!(at(&set("2024-06-03 10:00")), ms("2024-06-03T14:00:00Z"));
    assert_eq!(
        at(&set("soon")),
        ms("2024-01-31T14:30:00Z"),
        "nonsense keeps the default"
    );
}

#[test]
fn deep_ternaries() {
    let src = format!(
        "plot({}-1{})",
        (0..200)
            .map(|k| format!("(bar_index == {k} ? {k} : "))
            .collect::<String>(),
        ")".repeat(200)
    );
    assert_eq!(last(&src), 39.0);
}

#[test]
fn sec_vwap_resets_by_its_own_sessions() {
    // Daily VWAP seen from 4h bars: each context has its own days.
    let b = hours(96);
    let p = on(
        6,
        "plot(request.security(syminfo.tickerid, \"240\", ta.vwap(close)))",
        &b,
        H1,
        Env::default(),
    );
    let four: Vec<Bar> = b
        .chunks(4)
        .map(|c| Bar {
            time: c[0].time,
            open: c[0].open,
            high: c.iter().map(|x| x.high).fold(f64::MIN, f64::max),
            low: c.iter().map(|x| x.low).fold(f64::MAX, f64::min),
            close: c[3].close,
            volume: c.iter().map(|x| x.volume).sum(),
        })
        .collect();
    let h4 = on(
        6,
        "plot(ta.vwap(close))",
        &four,
        Timeframe::new(4, Unit::Hour),
        Env::default(),
    );
    for k in 0..four.len() {
        assert!(
            eq(p[0][k * 4 + 3], h4[0][k]),
            "4h bar {k}: {} vs {}",
            p[0][k * 4 + 3],
            h4[0][k]
        );
    }
}

#[test]
fn sec_shadowed_name_means_the_right_variable() {
    // `f`'s own `src` isn't the global one.
    let p = run(
        "a = close\nb = a * 3\nf() =>\n    a = 1.0\n    request.security(syminfo.tickerid, \"240\", b) + a * 0\nplot(f())\nplot(request.security(syminfo.tickerid, \"240\", close * 3))",
        60,
    );
    same(&p[0], &p[1]);
}

/// The same times with a random walk's prices: gaps, flat bars, no volume.
fn walk(b: &[Bar]) -> Vec<Bar> {
    let mut seed = 0x2545_F491_4F6C_DD1Du64;
    let mut rnd = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed >> 11) as f64 / (1u64 << 53) as f64
    };
    let mut x = 100.0;
    b.iter()
        .map(|b| {
            let open = if rnd() < 0.1 {
                x * (1.0 + (rnd() - 0.5) * 0.02)
            } else {
                x
            };
            let close = if rnd() < 0.03 {
                open
            } else {
                open * (1.0 + (rnd() - 0.5) * 0.04)
            };
            let flat = close == open && rnd() < 0.5;
            let up = if flat { 0.0 } else { rnd() * 0.01 };
            let down = if flat { 0.0 } else { rnd() * 0.01 };
            x = close;
            Bar {
                time: b.time,
                open,
                high: open.max(close) * (1.0 + up),
                low: open.min(close) * (1.0 - down),
                close,
                volume: if rnd() < 0.02 { 0.0 } else { (rnd() * 1000.0).round() },
            }
        })
        .collect()
}

fn nse_days(n: usize) -> Vec<Bar> {
    weekdays("2024-01-01T00:00:00Z", n)
        .iter()
        .flat_map(|d| (0..25).map(move |k| bar(d.time + 13_500_000 + k * 900_000, 100.0)))
        .collect()
}

/// Feeds the last bars one at a time with ticks in between, as a live chart
/// does. Every closed bar must come out as it did from history.
fn live_differs(s: &Script, b: &[Bar], tf: Timeframe, whole: &opentern_series::study::Output) -> Option<String> {
    let tz = jiff::tz::TimeZone::get(&s.prog.env.timezone).unwrap();
    let mut inst = opentern_series::study::Instance::with_factory(s.meta(), &Inputs::default(), s.factory());
    let n = b.len();
    let from = n - 40;
    for k in from..=n {
        let mut part = b[..k].to_vec();
        let sess = opentern_series::study::sessions(&part, tf, &tz);
        if k == from {
            inst.recompute(&part, &sess, tf);
        }
        // Two ticks on the way to the close.
        let end = part[k - 1];
        for f in [0.3, 0.8] {
            let t = part.last_mut().unwrap();
            t.close = end.open + (end.close - end.open) * f;
            t.high = end.open.max(t.close);
            t.low = end.open.min(t.close);
            t.volume = end.volume * f;
            inst.update(&part, &sess, tf);
        }
        *part.last_mut().unwrap() = end;
        inst.update(&part, &sess, tf);
        if let Some(e) = inst
            .latest_state()
            .and_then(|a| a.downcast_ref::<crate::PineStudy>())
            .and_then(|p| p.error())
        {
            return Some(format!("live: {e}"));
        }
    }
    for (p, (a, w)) in inst.out.plots.iter().zip(&whole.plots).enumerate() {
        for i in 0..n - 1 {
            if !eq(a[i], w[i]) {
                return Some(format!("plot {p} bar {i} of {n}: live {} history {}", a[i], w[i]));
            }
        }
    }
    None
}

fn nse_15m() -> Vec<Bar> {
    // 25 bars a day, 09:15 to 15:30 in Kolkata (03:45 to 10:00 UTC).
    let mut out = Vec::new();
    for d in ["2024-01-01", "2024-01-02", "2024-01-03"] {
        let open = ms(&format!("{d}T03:45:00Z"));
        for k in 0..25 {
            out.push(bar(open + k * 900_000, 100.0 + out.len() as f64));
        }
    }
    out
}

#[test]
fn sec_hours_start_at_the_session_open() {
    // NSE's 60-minute bars are 09:15, 10:15, … 15:15 (to the 15:30 close).
    let b = nse_15m();
    let p = on(
        6,
        "plot(request.security(syminfo.tickerid, \"60\", open))\nplot(request.security(syminfo.tickerid, \"60\", close))\nplot(timeframe.change(\"60\") ? 1 : 0)\nplot(time(\"60\"))",
        &b,
        M15,
        india(),
    );
    // 10:00 is the last bar of the 09:15 hour.
    assert_eq!(p[0][3], b[0].open);
    assert_eq!(p[1][3], b[3].close);
    assert_eq!(p[0][7], b[4].open);
    // The last, short hour ends with the session.
    assert_eq!(p[0][24], b[24].open);
    assert_eq!(p[1][24], b[24].close);
    assert_eq!((p[2][4], p[2][3]), (1.0, 0.0));
    assert_eq!(p[3][5], b[4].time as f64);
}

// ------------------------------------------------------------ dates on daily bars

#[test]
fn us_daily_bars_have_their_own_dates() {
    // Tue 2 Jan 2024 …
    let b = weekdays("2024-01-02T00:00:00Z", 30);
    let p = on(
        6,
        "plot(dayofmonth)\nplot(dayofweek)\nplot(month)\nplot(hour)\nplot(time >= timestamp(2024, 1, 2) ? 1 : 0)\n\
         plot(timeframe.change(\"W\") ? 1 : 0)\nplot(timeframe.change(\"M\") ? 1 : 0)\nplot(time_close)\nplot(time(\"W\"))\nplot(dayofmonth(time))\nplot(time)",
        &b,
        D1,
        us(),
    );
    assert_eq!(p[0][0], 2.0);
    assert_eq!(p[1][0], 3.0, "a Tuesday");
    assert_eq!(p[2][22], 2.0, "1 Feb is in February");
    assert_eq!(p[3][0], 9.0, "opens at 09:30");
    assert_eq!(p[4][0], 1.0, "a backtest from 2 Jan includes 2 Jan");
    // Mon 8 Jan is the 5th bar.
    assert_eq!((p[5][3], p[5][4], p[5][5]), (0.0, 1.0, 0.0));
    assert_eq!((p[6][21], p[6][22]), (0.0, 1.0));
    assert_eq!(p[7][0] as i64, ms("2024-01-02T21:00:00Z"), "closes at 16:00");
    assert_eq!(p[9][4], 8.0);
    assert_eq!(p[10][0] as i64, ms("2024-01-02T14:30:00Z"));
}

#[test]
fn us_weeks_from_daily_bars() {
    let b = weekdays("2024-01-01T00:00:00Z", 30);
    let p = on(
        6,
        "plot(request.security(syminfo.tickerid, \"W\", open))\nplot(request.security(syminfo.tickerid, \"W\", close))\nplot(request.security(syminfo.tickerid, \"M\", close))",
        &b,
        D1,
        us(),
    );
    // Friday 5 Jan closes the first week: Monday's open, Friday's close.
    assert_eq!(p[0][4], b[0].open);
    assert_eq!(p[1][4], b[4].close);
    assert_eq!(p[0][9], b[5].open);
    assert_eq!(p[1][8], b[4].close, "Thursday still shows last week");
    // January ends on Wednesday the 31st, the 23rd weekday.
    assert_eq!(p[2][22], b[22].close);
    assert!(p[2][21].is_nan());
}

#[test]
fn sessions_have_a_last_bar() {
    let b = nse_15m();
    let p = on(
        6,
        "plot(session.islastbar ? 1 : 0)\nplot(session.isfirstbar ? 1 : 0)",
        &b,
        M15,
        india(),
    );
    assert_eq!((p[0][23], p[0][24], p[0][49]), (0.0, 1.0, 1.0));
    assert_eq!((p[1][0], p[1][1], p[1][25]), (1.0, 0.0, 1.0));
}

#[test]
fn last_bar_time_is_known_on_every_bar() {
    let p = run("plot(last_bar_time)", 10);
    assert_eq!(p[0][0] as i64, hours(10)[9].time);
}

#[test]
fn session_strings() {
    let p = run(
        "plot(na(time(timeframe.period, \"0200-0400,0800-1000\", \"UTC\")) ? 0 : 1)\nplot(na(time(timeframe.period, \"0000-0000\", \"UTC\")) ? 0 : 1)\n\
         plot(na(time(timeframe.period, \"2200-0200:2\", \"UTC\")) ? 0 : 1)",
        48,
    );
    let want: Vec<f64> = (0..24)
        .map(|h| if [2, 3, 8, 9].contains(&h) { 1.0 } else { 0.0 })
        .collect();
    assert_eq!(&p[0][..24], &want[..]);
    assert!(p[1].iter().all(|x| *x == 1.0));
    // 1 Jan 2024 is a Monday; Monday's overnight session starts on Sunday.
    assert_eq!((p[2][1], p[2][23], p[2][25]), (1.0, 0.0, 0.0));
}

// ------------------------------------------------------------ lengths that change

#[test]
fn series_lengths() {
    let n = 60;
    let p = run(
        "len = 3 + bar_index % 4\nplot(ta.sma(close, len))\nplot(ta.highest(high, len))\nplot(ta.lowest(low, len))\nplot(ta.stdev(close, len))\n\
         plot(ta.wma(close, len))\nplot(math.sum(close, len))\nplot(ta.change(close, len))\nplot(ta.highestbars(high, len))\nplot(ta.linreg(close, len, 0))\nplot(ta.median(close, len))\nplot(ta.vwma(close, len))\nplot(ta.roc(close, len))\nplot(ta.mom(close, len))\nplot(ta.percentrank(close, len))\nplot(ta.cci(close, len))\nplot(ta.mfi(close, len))\nplot(ta.stoch(close, high, low, len))\nplot(ta.variance(close, len))\nplot(ta.dev(close, len))\nplot(ta.rising(close, len) ? 1 : 0)\nplot(ta.correlation(close, high * high, len))\nplot(ta.range(close, len))",
        n,
    );
    // The same with fixed lengths, picked bar by bar.
    let fixed: Vec<Vec<Vec<f64>>> = (3..7)
        .map(|k| {
            run(
                &format!(
                    "plot(ta.sma(close, {k}))\nplot(ta.highest(high, {k}))\nplot(ta.lowest(low, {k}))\nplot(ta.stdev(close, {k}))\n\
                     plot(ta.wma(close, {k}))\nplot(math.sum(close, {k}))\nplot(ta.change(close, {k}))\nplot(ta.highestbars(high, {k}))\nplot(ta.linreg(close, {k}, 0))\nplot(ta.median(close, {k}))\nplot(ta.vwma(close, {k}))\nplot(ta.roc(close, {k}))\nplot(ta.mom(close, {k}))\nplot(ta.percentrank(close, {k}))\nplot(ta.cci(close, {k}))\nplot(ta.mfi(close, {k}))\nplot(ta.stoch(close, high, low, {k}))\nplot(ta.variance(close, {k}))\nplot(ta.dev(close, {k}))\nplot(ta.rising(close, {k}) ? 1 : 0)\nplot(ta.correlation(close, high * high, {k}))\nplot(ta.range(close, {k}))"
                ),
                n,
            )
        })
        .collect();
    for s in 0..p.len() {
        for i in 10..n {
            let want = fixed[i % 4][s][i];
            assert!((p[s][i] - want).abs() < 1e-6, "plot {s} bar {i}: {} vs {want}", p[s][i]);
        }
    }
}

// ------------------------------------------------------------ inputs and declarations

#[test]
fn input_arguments_from_named_constants() {
    let s = load(
        6,
        "G = \"Main\"\nconst string T = \"Len\"\nA = \"one\"\nB = \"two\"\nD = 7 * 2\nn = input.int(D, T, minval = 1, group = G)\nm = input.string(A, \"Mode\", options = [A, B], group = G + \" 2\")\nplot(n)",
        Env::default(),
    );
    let i = &s.meta().inputs;
    assert_eq!((i[0].title, i[0].group), ("Len", Some("Main")));
    assert_eq!(i[1].group, Some("Main 2"));
    assert_eq!(s.study(&Inputs::default()).run(&hours(2), H1).plots[0][0], 14.0);
}

#[test]
fn an_input_is_not_a_constant() {
    let src = "//@version=6\nindicator(\"T\")\ng = input.string(\"a\")\nn = input.int(1, group = g)\nplot(n)";
    assert!(compile(src, "t.pine", Env::default()).is_err());
}

#[test]
fn input_positions_follow_the_manual() {
    let s = load(
        6,
        "a = input.int(14, \"A\", 1, 100, 1, \"a tip\", \"line\", \"grp\")\nb = input.bool(true, \"B\", \"tip\", \"line\", \"grp\")\nc = input.color(color.red, \"C\", \"tip\", \"line\", \"grp\")\nd = input.source(close, \"D\", \"tip\", \"line\", \"grp\")\ne = input.string(\"x\", \"E\", [\"x\", \"y\"], \"tip\", \"line\", \"grp\")\nf = input.int(2, \"F\", [1, 2, 3], \"tip\", \"line\", \"grp\")\ng = input.timeframe(\"D\", \"G\", [\"D\", \"W\"], \"tip\", \"line\", \"grp\")\nh = input.float(1.5, \"H\", 0.5, 3, 0.1, \"tip\")\nplot(a)",
        Env::default(),
    );
    let i = &s.meta().inputs;
    assert_eq!(i.len(), 8);
    for k in [0, 1, 2, 3, 4, 5, 6] {
        assert_eq!(i[k].group, Some("grp"), "{}", i[k].title);
    }
}

#[test]
fn input_time_takes_timestamps() {
    let p = run(
        "t = input.time(timestamp(\"2024-01-02\"), \"From\")\nplot(t)\nu = input.time(timestamp(\"20 Feb 2024 00:00 +0000\"))\nplot(u)\nw = input.time(timestamp(2024, 1, 2))\nplot(w)",
        2,
    );
    assert_eq!(p[0][0] as i64, ms("2024-01-02T00:00:00Z"));
    assert_eq!(p[1][0] as i64, ms("2024-02-20T00:00:00Z"));
}

#[test]
fn timestamp_text_is_gmt() {
    let p = on(
        6,
        "plot(timestamp(\"2024-01-02\"))\nplot(timestamp(\"Feb 01 2024 22:10:05\"))\nplot(timestamp(\"2024-02-01T22:10:05+03:00\"))\nplot(timestamp(\"01 Jan 2024 09:30 GMT-0500\"))\nplot(timestamp(\"1 Jan 2024\"))\nplot(timestamp(\"America/New_York\", 2024, 1, 2, 9, 30))\nplot(timestamp(year = 2024, month = 3, day = 4, hour = 5, minute = 6))\nplot(timestamp(\"2024-01-02 15:30\"))",
        &hours(2),
        H1,
        us(),
    );
    assert_eq!(p[0][0] as i64, ms("2024-01-02T00:00:00Z"));
    assert_eq!(p[1][0] as i64, ms("2024-02-01T22:10:05Z"));
    assert_eq!(p[2][0] as i64, ms("2024-02-01T19:10:05Z"));
    assert_eq!(p[3][0] as i64, ms("2024-01-01T14:30:00Z"));
    assert_eq!(p[4][0] as i64, ms("2024-01-01T00:00:00Z"));
    assert_eq!(p[5][0] as i64, ms("2024-01-02T14:30:00Z"));
    assert_eq!(p[6][0] as i64, ms("2024-03-04T10:06:00Z"));
    assert_eq!(p[7][0] as i64, ms("2024-01-02T15:30:00Z"));
}

#[test]
fn output_positions_follow_the_manual() {
    // …, text, textcolor, editable, size
    let s = load(
        6,
        "plotshape(close > open, \"S\", shape.circle, location.top, color.red, 0, \"x\", color.white, true, size.huge)\nplotchar(true, \"C\", \"x\", location.top, color.red, 0, \"\", color.white, true, size.tiny)\nbgcolor(color.red, 0, true, 10, \"bg\")\nhline(50, \"h\", color.red, hline.style_solid, 2, true)\np = plot(close, \"P\", color.red, 2, plot.style_line, false, 0.0, 0, false, true)\nq = plot(open)\nfill(p, q, color.red, \"f\", true, true)\nplotcandle(open, high, low, close, \"k\", color.red, color.blue, true, 100, color.green)\nplotbar(open, high, low, close, \"b\", color.red, true, 5)",
        Env::default(),
    );
    assert_eq!(s.meta().plots[0].shape.as_ref().unwrap().size, 26.0);
    assert_eq!(s.meta().plots[1].shape.as_ref().unwrap().size, 6.0);
}

#[test]
fn gradient_fills() {
    let s = load(
        6,
        "p = plot(close)\nq = plot(open)\nfill(p, q, 100, 70, top_color = color.new(color.green, 0), bottom_color = color.new(color.green, 100), title = \"G\")\nfill(p, q, close, open, color.red, color.blue, \"H\")\nfill(plot1 = p, plot2 = q, color = color.red)",
        Env::default(),
    );
    assert_eq!(s.meta().fills.len(), 3);
    assert_eq!((s.meta().fills[0].title, s.meta().fills[1].title), ("G", "H"));
    let mut st = s.study(&Inputs::default());
    st.run(&hours(3), H1);
    assert!(st.error().is_none());
}

#[test]
fn declaration_positions() {
    let s = compile(
        "//@version=6\nindicator(\"T\", \"t\", true, format.price, 4, scale.right, 500, \"\", true, false, 100, 200, 300)\nplot(close)",
        "t",
        Env::default(),
    )
    .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(s.meta().format, opentern_series::study::Format::Fixed(4));
    let s = compile(
        "//@version=6\nCAP = 5000\nstrategy(\"T\", initial_capital = CAP * 2, default_qty_value = 10)\nplot(close)",
        "t",
        Env::default(),
    )
    .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(s.strategy_props().unwrap()["initial_capital"], 10000);
}

#[test]
fn an_indicator_on_another_timeframe() {
    // `timeframe = "240"` runs the whole script on 4h bars.
    let src = "//@version=6\nindicator(\"T\", timeframe = \"240\", timeframe_gaps = false)\nplot(ta.sma(close, 3))";
    let want = plot(
        "plot(request.security(syminfo.tickerid, \"240\", ta.sma(close, 3)))",
        60,
    );
    match compile(src, "t", Env::default()) {
        Ok(s) => same(&s.study(&Inputs::default()).run(&hours(60), H1).plots[0], &want),
        Err(e) => panic!("{e}"),
    }
}

#[test]
fn hlines_take_constants_and_inputs() {
    let s = load(
        6,
        "L = 35 * 2\nn = input.int(30)\nhline(L)\nhline(100 - n)\nhline(L + 5, \"x\")",
        Env::default(),
    );
    let h = &s.meta().hlines;
    assert_eq!((h[0].value, h[1].value, h[2].value), (70.0, 70.0, 75.0));
}

#[test]
fn fill_takes_plots_directly() {
    let s = load(
        6,
        "fill(plot(close), plot(open), color.red)\nfill(hline(70), hline(30), color.blue)",
        Env::default(),
    );
    assert_eq!(s.meta().fills.len(), 2);
}

// ------------------------------------------------------------ language

#[test]
fn soft_keywords_as_names() {
    assert_eq!(
        last(
            "ma(float src, int length, string type) =>\n    type == \"SMA\" ? ta.sma(src, length) : src\nstring type = \"SMA\"\nplot(ma(close, 1, type) - close)"
        ),
        0.0
    );
}

#[test]
fn loops_have_values() {
    let v = plot(
        "x = for i = 1 to 3\n    i * 10\ny = while false\n    1\nz = for v in array.from(1, 2, 7)\n    v\nplot(x)\nplot(z)",
        1,
    );
    assert_eq!(v[0], 30.0);
}

#[test]
fn several_statements_on_one_line() {
    assert_eq!(last("f(x) => a = x * 2, a + 1\nplot(f(3))"), 7.0);
    assert_eq!(
        last("a = 0\nb = 0\nswitch\n    close > 0 => a := 1, b := 2\n    => a := 9\nplot(a * 10 + b)"),
        12.0
    );
    assert_eq!(
        last("type P\n    float x = 0\np = P.new()\nswitch\n    close > 0 => p.x := 5\nplot(p.x)"),
        5.0
    );
}

#[test]
fn byte_order_mark_and_comments() {
    assert!(
        compile(
            "\u{feff}//@version=6 // latest\nindicator(\"T\")\nplot(close)",
            "t",
            Env::default()
        )
        .is_ok()
    );
    // The version decides how `and` works.
    let src = "//@version=6 // c\nindicator(\"T\")\nn = 0\nf() =>\n    1\nplot(5 / 2)";
    let mut st = compile(src, "t", Env::default()).unwrap().study(&Inputs::default());
    assert_eq!(st.run(&hours(1), H1).plots[0][0], 2.5);
}

#[test]
fn v5_divides_constant_ints() {
    assert_eq!(on(5, "a = 5\nb = 2\nplot(a / b)\nint n = 7\nplot(n / 2)\nfloat x = 5\nplot(x / 2)\nvar k = 5\nk := 7\nplot(k / 2)", &hours(1), H1, Env::default()).iter().map(|p| p[0]).collect::<Vec<_>>(), [2.0, 3.0, 2.5, 3.5]);
    assert_eq!(
        on(6, "a = 5\nb = 2\nplot(a / b)", &hours(1), H1, Env::default())[0][0],
        2.5
    );
}

#[test]
fn types_with_varip_and_default_copies() {
    assert_eq!(
        last(
            "type P\n    varip int n = 3\n    float y = close\nP p = P.new()\nq = P.copy(p)\nq.n := 8\nplot(p.n * 10 + q.n)"
        ),
        38.0
    );
    assert_eq!(last("type P\n    float y = close\np = P.new()\nplot(p.y - close)"), 0.0);
}

#[test]
fn methods_pick_by_type() {
    assert_eq!(
        last(
            "method kind(array<float> a) => 1\nmethod kind(map<string, float> m) => 2\nmethod kind(string s) => 3\nmethod kind(float x) => 4\nmethod kind(bool b) => 5\nmethod kind(color c) => 6\na = array.new<float>(1, 1.0)\nm = map.new<string, float>()\ns = \"x\"\nb = close > 0\nplot(a.kind() * 100000 + m.kind() * 10000 + s.kind() * 1000 + close.kind() * 100 + b.kind() * 10)"
        ),
        123450.0
    );
}

#[test]
fn overloads_pick_by_type() {
    assert_eq!(
        last(
            "f(string s) => 1\nf(float x) => 2\nf(bool b) => 3\nf(array<int> a) => 4\nx = array.new<int>(2, 1)\nplot(f(\"a\") * 1000 + f(close) * 100 + f(close > open) * 10 + f(x))"
        ),
        1234.0
    );
    assert_eq!(
        last("f(string s) => 1\nf(float x) => 2\nplot(f(str.tostring(close)) * 10 + f(ta.sma(close, 2)))"),
        12.0
    );
}

#[test]
fn library_constants() {
    let env = Env {
        libraries: vec![(
            "Lib".into(),
            "//@version=6\nlibrary(\"Lib\")\nFACTOR = 3\nexport const int TEN = 10\nexport times(float x) => x * FACTOR".into(),
        )],
        ..Env::default()
    };
    let p = on(
        6,
        "import me/Lib/1 as lib\nplot(lib.times(2) + lib.TEN)",
        &hours(1),
        H1,
        env,
    );
    assert_eq!(p[0][0], 16.0);
}

#[test]
fn for_in_follows_the_array() {
    assert_eq!(
        last("a = array.from(1, 2, 3, 4)\nfor [i, x] in a\n    if x == 2\n        a.remove(i)\nplot(a.size())"),
        3.0
    );
}

#[test]
fn history_of_a_constant_starts_at_na() {
    let v = plot("plot(na(5[1]) ? 1 : 0)", 2);
    assert_eq!(v[1], 0.0);
}

// ------------------------------------------------------------ text

#[test]
fn number_text() {
    assert_eq!(text(6, "out = str.tostring(0.5, \"#.##\")"), "0.5");
    assert_eq!(text(6, "out = str.tostring(-0.00000000001)"), "0");
    assert_eq!(text(6, "out = str.tostring(array.from(1, 2, 3))"), "[1, 2, 3]");
    assert_eq!(text(6, "out = str.format(\"{0}\", 1234.56789)"), "1,234.568");
    assert_eq!(text(6, "out = str.format(\"it''s '{0}' {0}\", 5)"), "it's {0} 5");
    assert_eq!(text(6, "out = str.format(\"{0,number,#.#}\", 3.14159)"), "3.1");
    assert_eq!(
        text(
            6,
            "out = str.join(str.split(\"a,b\", \",\"), \"-\")"
                .replace("str.join", "array.join")
                .as_str()
        ),
        "a-b"
    );
    assert_eq!(text(6, "out = str.match(\"It's 5 o'clock\", \"[0-9]+\")"), "5");
}

// ------------------------------------------------------------ colours

#[test]
fn colours_follow_the_version() {
    assert_eq!(text(5, "out = str.tostring(color.r(color.red))"), "255");
    assert_eq!(
        text(
            6,
            "out = str.tostring(color.r(color.red)) + \" \" + str.tostring(color.b(color.yellow))"
        ),
        "242 53"
    );
}

// ------------------------------------------------------------ drawings

#[test]
fn drawings_may_start_without_coordinates() {
    let mut st = load(
        6,
        "var line l = line.new(na, na, na, na)\nvar label b = label.new(na, na, \"x\")\nvar box x = box.new(na, na, na, na)\nplot(na(line.get_x1(l)) ? 1 : 0)\nif barstate.islast\n    line.set_xy1(l, bar_index - 1, 1)\n    line.set_xy2(l, bar_index, 2)",
        Env::default(),
    )
    .study(&Inputs::default());
    let out = st.run(&hours(5), H1);
    assert!(st.error().is_none(), "{:?}", st.error());
    assert_eq!(out.plots[0][0], 1.0);
    let d = st.drawings();
    assert_eq!((d.lines.len(), d.labels.len(), d.boxes.len()), (1, 0, 0));
}

#[test]
fn a_dead_handle_never_reaches_another_drawing() {
    let mut st = load(
        6,
        "var line a = line.new(0, 1, 1, 1)\nif bar_index == 1\n    line.delete(a)\nvar line b = na\nif bar_index == 2\n    b := line.new(0, 2, 1, 2)\nif bar_index == 3\n    line.delete(a)\n    line.set_y1(a, 99)",
        Env::default(),
    )
    .study(&Inputs::default());
    st.run(&hours(6), H1);
    let d = st.drawings();
    assert_eq!(d.lines.len(), 1, "b is still there");
    assert_eq!(d.lines[0].y1, 2.0);
}

#[test]
fn one_table_per_corner() {
    let mut st = load(6, "t = table.new(position.top_right, 1, 1)\ntable.cell(t, 0, 0, str.tostring(bar_index))\ntable.cell_set_text(t, 0, 0, text = \"n\" + str.tostring(bar_index))\ntable.cell_set_bgcolor(t, 0, 0, bgcolor = color.red)", Env::default()).study(&Inputs::default());
    st.run(&hours(6), H1);
    assert!(st.error().is_none());
    let d = st.drawings();
    assert_eq!(d.tables.len(), 1);
    assert_eq!(&*d.tables[0].cells[0].text, "n5");
}

// ------------------------------------------------------------ alerts

#[test]
fn alert_messages_name_plots() {
    let mut st = load(6, "plot(close, \"Fast\")\nplot(open + 1)\nalertcondition(bar_index == 2, \"a\", \"{{plot_0}} {{plot(\\\"Fast\\\")}} {{plot_1}}\")", Env::default()).study(&Inputs::default());
    let b = hours(4);
    st.run(&b, H1);
    let a = &st.alerts()[0].message;
    let want = crate::value::fmt_num(b[2].close);
    assert!(a.starts_with(&format!("{want} {want} ")), "{a}");
    assert!(!a.contains("{{"), "{a}");
}

#[test]
fn alert_text_from_named_constants() {
    let mut st = load(
        6,
        "M = \"hello\"\nT = \"title\"\nalertcondition(true, T, M + \" you\")",
        Env::default(),
    )
    .study(&Inputs::default());
    st.run(&hours(1), H1);
    assert_eq!(
        (st.alerts()[0].title.as_str(), st.alerts()[0].message.as_str()),
        ("title", "hello you")
    );
}

// ------------------------------------------------------------ strategies

fn trades(body: &str, n: usize) -> Vec<(i64, f64)> {
    let src =
        format!("//@version=6\nstrategy(\"T\", default_qty_type = strategy.fixed, default_qty_value = 3)\n{body}");
    let mut st = compile(&src, "t", Env::default())
        .unwrap_or_else(|e| panic!("{e}\n{src}"))
        .study(&Inputs::default());
    st.run(&hours(n), H1);
    if let Some(e) = st.error() {
        panic!("{e}");
    }
    let b = st.broker().unwrap();
    b.trades
        .iter()
        .map(|t| (t.entry_bar as i64, if t.long { t.qty } else { -t.qty }))
        .chain(
            b.lots
                .iter()
                .map(|l| (l.bar as i64, if l.long { l.qty } else { -l.qty })),
        )
        .collect()
}

#[test]
fn a_quantity_of_zero_places_nothing() {
    assert_eq!(
        trades(
            "if bar_index == 2\n    strategy.entry(\"L\", strategy.long, qty = 0)",
            8
        ),
        []
    );
    assert_eq!(
        trades(
            "if bar_index == 2\n    strategy.entry(\"L\", strategy.long, qty = na)",
            8
        ),
        [(3, 3.0)]
    );
    assert_eq!(
        trades(
            "if bar_index == 2\n    strategy.entry(\"L\", strategy.long)\nif bar_index == 5\n    strategy.close(\"L\", qty = 0)",
            9
        ),
        [(3, 3.0)]
    );
}

#[test]
fn risk_rules_are_kept() {
    assert_eq!(
        trades(
            "strategy.risk.max_position_size(2)\nif bar_index == 2\n    strategy.entry(\"L\", strategy.long, qty = 5)",
            8
        ),
        [(3, 2.0)]
    );
    // A drawdown rule stops trading for good.
    let t = trades(
        "strategy.risk.max_drawdown(0.0000001, strategy.percent_of_equity)\nif bar_index % 4 == 1\n    strategy.entry(\"L\", strategy.long)\nif bar_index % 4 == 3\n    strategy.close(\"L\")",
        40,
    );
    assert_eq!(t.len(), 1, "{t:?}");
}

// ------------------------------------------------------------ every name in the manual

#[test]
fn every_built_in_is_known() {
    const VARS: &str = "ask bar_index bid close dayofmonth dayofweek high hl2 hlc3 hlcc4 hour last_bar_index last_bar_time low minute month ohlc4 open second time time_close time_tradingday timenow volume weekofyear year na \
        barstate.isconfirmed barstate.isfirst barstate.ishistory barstate.islast barstate.islastconfirmedhistory barstate.isnew barstate.isrealtime box.all chart.bg_color chart.fg_color chart.is_heikinashi chart.is_kagi chart.is_linebreak chart.is_pnf chart.is_range chart.is_renko chart.is_standard chart.left_visible_bar_time chart.right_visible_bar_time dividends.future_amount dividends.future_ex_date dividends.future_pay_date earnings.future_eps earnings.future_period_end_time earnings.future_revenue earnings.future_time label.all line.all linefill.all polyline.all table.all \
        session.isfirstbar session.isfirstbar_regular session.islastbar session.islastbar_regular session.ismarket session.ispostmarket session.ispremarket \
        syminfo.basecurrency syminfo.country syminfo.currency syminfo.current_contract syminfo.description syminfo.employees syminfo.expiration_date syminfo.industry syminfo.isin syminfo.main_tickerid syminfo.mincontract syminfo.minmove syminfo.mintick syminfo.pointvalue syminfo.prefix syminfo.pricescale syminfo.recommendations_buy syminfo.recommendations_buy_strong syminfo.recommendations_date syminfo.recommendations_hold syminfo.recommendations_sell syminfo.recommendations_sell_strong syminfo.recommendations_total syminfo.root syminfo.sector syminfo.session syminfo.shareholders syminfo.shares_outstanding_float syminfo.shares_outstanding_total syminfo.target_price_average syminfo.target_price_date syminfo.target_price_estimates syminfo.target_price_high syminfo.target_price_low syminfo.target_price_median syminfo.ticker syminfo.tickerid syminfo.timezone syminfo.type syminfo.volumetype \
        ta.accdist ta.iii ta.nvi ta.obv ta.pvi ta.pvt ta.tr ta.vwap ta.wad ta.wvad timeframe.isdaily timeframe.isdwm timeframe.isintraday timeframe.isminutes timeframe.ismonthly timeframe.isseconds timeframe.isticks timeframe.isweekly timeframe.main_period timeframe.multiplier timeframe.period";
    const STRAT: &str = "strategy.account_currency strategy.avg_losing_trade strategy.avg_losing_trade_percent strategy.avg_trade strategy.avg_trade_percent strategy.avg_winning_trade strategy.avg_winning_trade_percent strategy.closedtrades strategy.closedtrades.first_index strategy.equity strategy.eventrades strategy.grossloss strategy.grossloss_percent strategy.grossprofit strategy.grossprofit_percent strategy.initial_capital strategy.losstrades strategy.margin_liquidation_price strategy.max_contracts_held_all strategy.max_contracts_held_long strategy.max_contracts_held_short strategy.max_drawdown strategy.max_drawdown_percent strategy.max_runup strategy.max_runup_percent strategy.netprofit strategy.netprofit_percent strategy.openprofit strategy.openprofit_percent strategy.opentrades strategy.opentrades.capital_held strategy.position_avg_price strategy.position_entry_name strategy.position_size strategy.wintrades";
    const FNS: &str = "alert array.abs array.avg array.binary_search array.binary_search_leftmost array.binary_search_rightmost array.clear array.concat array.copy array.covariance array.every array.fill array.first array.from array.get array.includes array.indexof array.insert array.join array.last array.lastindexof array.max array.median array.min array.mode array.new array.new_bool array.new_box array.new_color array.new_float array.new_int array.new_label array.new_line array.new_linefill array.new_string array.new_table array.percentile_linear_interpolation array.percentile_nearest_rank array.percentrank array.pop array.push array.range array.remove array.reverse array.set array.shift array.size array.slice array.some array.sort array.sort_indices array.standardize array.stdev array.sum array.unshift array.variance bool box box.copy box.delete box.get_bottom box.get_left box.get_right box.get_top box.new box.set_bgcolor box.set_border_color box.set_border_style box.set_border_width box.set_bottom box.set_bottom_right_point box.set_extend box.set_left box.set_lefttop box.set_right box.set_rightbottom box.set_text box.set_text_color box.set_text_font_family box.set_text_formatting box.set_text_halign box.set_text_size box.set_text_valign box.set_text_wrap box.set_top box.set_top_left_point box.set_xloc chart.point.copy chart.point.from_index chart.point.from_time chart.point.new chart.point.now color color.b color.from_gradient color.g color.new color.r color.rgb color.t dayofmonth dayofweek fixnan float hour int label label.copy label.delete label.get_text label.get_x label.get_y label.new label.set_color label.set_point label.set_size label.set_style label.set_text label.set_text_font_family label.set_text_formatting label.set_textalign label.set_textcolor label.set_tooltip label.set_x label.set_xloc label.set_xy label.set_y label.set_yloc line line.copy line.delete line.get_price line.get_x1 line.get_x2 line.get_y1 line.get_y2 line.new line.set_color line.set_extend line.set_first_point line.set_second_point line.set_style line.set_width line.set_x1 line.set_x2 line.set_xloc line.set_xy1 line.set_xy2 line.set_y1 line.set_y2 linefill linefill.delete linefill.get_line1 linefill.get_line2 linefill.new linefill.set_color log.error log.info log.warning map.clear map.contains map.copy map.get map.keys map.new map.put map.put_all map.remove map.size map.values math.abs math.acos math.asin math.atan math.avg math.ceil math.cos math.exp math.floor math.log math.log10 math.max math.min math.pow math.random math.round math.round_to_mintick math.sign math.sin math.sqrt math.sum math.tan math.todegrees math.toradians matrix.add_col matrix.add_row matrix.avg matrix.col matrix.columns matrix.concat matrix.copy matrix.det matrix.diff matrix.eigenvalues matrix.eigenvectors matrix.elements_count matrix.fill matrix.get matrix.inv matrix.is_antidiagonal matrix.is_antisymmetric matrix.is_binary matrix.is_diagonal matrix.is_identity matrix.is_square matrix.is_stochastic matrix.is_symmetric matrix.is_triangular matrix.is_zero matrix.kron matrix.max matrix.median matrix.min matrix.mode matrix.mult matrix.new matrix.pinv matrix.pow matrix.rank matrix.remove_col matrix.remove_row matrix.reshape matrix.reverse matrix.row matrix.rows matrix.set matrix.sort matrix.submatrix matrix.sum matrix.swap_columns matrix.swap_rows matrix.trace matrix.transpose max_bars_back minute month na nz polyline.delete polyline.new request.currency_rate runtime.error second str.contains str.endswith str.format str.format_time str.length str.lower str.match str.pos str.repeat str.replace str.replace_all str.split str.startswith str.substring str.tonumber str.tostring str.trim str.upper strategy.cancel strategy.cancel_all strategy.close strategy.close_all strategy.closedtrades.commission strategy.closedtrades.entry_bar_index strategy.closedtrades.entry_comment strategy.closedtrades.entry_id strategy.closedtrades.entry_price strategy.closedtrades.entry_time strategy.closedtrades.exit_bar_index strategy.closedtrades.exit_comment strategy.closedtrades.exit_id strategy.closedtrades.exit_price strategy.closedtrades.exit_time strategy.closedtrades.max_drawdown strategy.closedtrades.max_drawdown_percent strategy.closedtrades.max_runup strategy.closedtrades.max_runup_percent strategy.closedtrades.profit strategy.closedtrades.profit_percent strategy.closedtrades.size strategy.convert_to_account strategy.convert_to_symbol strategy.default_entry_qty strategy.entry strategy.exit strategy.opentrades.commission strategy.opentrades.entry_bar_index strategy.opentrades.entry_comment strategy.opentrades.entry_id strategy.opentrades.entry_price strategy.opentrades.entry_time strategy.opentrades.max_drawdown strategy.opentrades.max_drawdown_percent strategy.opentrades.max_runup strategy.opentrades.max_runup_percent strategy.opentrades.profit strategy.opentrades.profit_percent strategy.opentrades.size strategy.order strategy.risk.allow_entry_in strategy.risk.max_cons_loss_days strategy.risk.max_drawdown strategy.risk.max_intraday_filled_orders strategy.risk.max_intraday_loss strategy.risk.max_position_size string syminfo.prefix syminfo.ticker ta.alma ta.atr ta.barssince ta.bb ta.bbw ta.cci ta.change ta.cmo ta.cog ta.correlation ta.cross ta.crossover ta.crossunder ta.cum ta.dev ta.dmi ta.ema ta.falling ta.highest ta.highestbars ta.hma ta.kc ta.kcw ta.linreg ta.lowest ta.lowestbars ta.macd ta.max ta.median ta.mfi ta.min ta.mode ta.mom ta.percentile_linear_interpolation ta.percentile_nearest_rank ta.percentrank ta.pivot_point_levels ta.pivothigh ta.pivotlow ta.range ta.rci ta.rising ta.rma ta.roc ta.rsi ta.sar ta.sma ta.stdev ta.stoch ta.supertrend ta.swma ta.tr ta.tsi ta.valuewhen ta.variance ta.vwap ta.vwma ta.wma ta.wpr table table.cell table.cell_set_bgcolor table.cell_set_height table.cell_set_text table.cell_set_text_color table.cell_set_text_font_family table.cell_set_text_formatting table.cell_set_text_halign table.cell_set_text_size table.cell_set_text_valign table.cell_set_tooltip table.cell_set_width table.clear table.delete table.merge_cells table.new table.set_bgcolor table.set_border_color table.set_border_width table.set_frame_color table.set_frame_width table.set_position ticker.heikinashi ticker.inherit ticker.modify ticker.new ticker.standard time time_close timeframe.change timeframe.from_seconds timeframe.in_seconds timestamp weekofyear year";
    let mut missing = Vec::new();
    for v in VARS.split_whitespace().chain(STRAT.split_whitespace()) {
        let known = crate::compile::Field::by_name(v).is_some()
            || v == "na"
            || crate::natives::lookup(v).is_some_and(|d| d.var)
            || crate::compile::constant(v).is_some();
        if !known {
            missing.push(format!("var {v}"));
        }
    }
    for f in FNS.split_whitespace() {
        if crate::natives::lookup(f).is_none() {
            missing.push(format!("fn {f}"));
        }
    }
    assert!(missing.is_empty(), "{} missing:\n{}", missing.len(), missing.join("\n"));
}

#[test]
fn forming_bar_matches_history() {
    // The last bar computed as it forms equals the same bar once closed.
    let body =
        "plot(request.security(syminfo.tickerid, \"240\", ta.sma(close, 3)))\nplot(ta.sma(close, 3 + bar_index % 2))";
    let b = hours(48);
    let s = load(6, body, Env::default());
    let mut whole = s.study(&Inputs::default());
    let out = whole.run(&b, H1);
    let mut inst = opentern_series::study::Instance::with_factory(s.meta(), &Inputs::default(), s.factory());
    inst.recompute(&b, &vec![false; b.len()], H1);
    let _ = whole.drawings();
    for k in 0..2 {
        assert!(eq(inst.out.plots[k][46], out.plots[k][46]));
        assert!(eq(inst.out.plots[k][47], out.plots[k][47]));
    }
    let _: Option<&dyn Study> = None;
}

#[test]
fn hlines_follow_their_inputs() {
    let s = load(
        6,
        "n = input.int(70, \"OB\")\nc = input.color(color.red, \"C\")\nhline(n + 5, color = c)\nplot(close)",
        Env::default(),
    );
    let st = s.study(&Inputs::default());
    assert_eq!(st.levels(), [(75.0, Some(0xF23645FF))]);
    let mut i = Inputs::default();
    i.values.insert("ob".into(), opentern_series::study::Value::Int(80));
    assert_eq!(s.study(&i).levels()[0].0, 85.0);
}

#[test]
fn background_offsets() {
    let mut st = load(
        6,
        "bgcolor(bar_index == 2 ? color.red : na, offset = 2)",
        Env::default(),
    )
    .study(&Inputs::default());
    let out = st.run(&hours(8), H1);
    assert_eq!(out.bg.iter().position(|c| *c != 0), Some(4));
}

/// Runs every script in `tests/scripts` (or `PINE_DIR`) on several kinds of
/// chart, from history and live.
#[test]
fn a_folder_of_scripts() {
    let dir = std::env::var("PINE_DIR").unwrap_or(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/scripts").into());
    let mut files: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "pine"))
        .collect();
    files.sort();
    let mut bad = 0;
    for f in files {
        let src = std::fs::read_to_string(&f).unwrap();
        for (name, b, tf, env) in [
            ("1h", walk(&hours(1500)), H1, Env::default()),
            ("US daily", walk(&weekdays("2020-01-01T00:00:00Z", 900)), D1, us()),
            ("NSE 15m", walk(&nse_days(60)), M15, india()),
        ] {
            let r = std::panic::catch_unwind(|| match compile(&src, "t", env) {
                Err(e) => Some(format!("compile: {e}")),
                Ok(s) => {
                    let mut st = s.study(&Inputs::default());
                    let out = st.run(&b, tf);
                    if let Some(e) = st.error() {
                        return Some(format!("runtime: {e}"));
                    }
                    if let Some(e) = live_differs(&s, &b, tf, &out) {
                        return Some(e);
                    }
                    let finite = out.plots.iter().filter(|p| p.iter().any(|v| v.is_finite())).count();
                    println!(
                        "   {} plots, {finite} with values, {} trades",
                        out.plots.len(),
                        st.broker().map_or(0, |b| b.trades.len())
                    );
                    None
                }
            });
            match r {
                Ok(None) => println!("ok  {} {name}", f.file_name().unwrap().to_string_lossy()),
                Ok(Some(e)) => {
                    bad += 1;
                    println!("BAD {} {name}: {e}", f.file_name().unwrap().to_string_lossy())
                }
                Err(_) => {
                    bad += 1;
                    println!("PANIC {} {name}", f.file_name().unwrap().to_string_lossy())
                }
            }
        }
    }
    assert_eq!(bad, 0);
}

#[test]
fn methods_on_a_drawing_that_is_na() {
    assert_eq!(
        last(
            "type Z\n    line l = na\nvar line a = na\nvar box b = na\nz = Z.new()\na.delete()\nb.set_right(5)\nz.l.set_x2(3)\nline.delete(a)\nplot(na(a.get_x1()) ? 1 : 0)"
        ),
        1.0
    );
    let src = "//@version=6\nindicator(\"T\")\narray<float> a = na\na.push(1)";
    let mut st = compile(src, "t", Env::default()).unwrap().study(&Inputs::default());
    st.run(&hours(2), H1);
    assert!(st.error().is_some());
}

#[test]
fn methods_on_a_na_object_pick_the_right_type() {
    assert_eq!(
        last(
            "type A\n    float x = 1\ntype B\n    float x = 1\nmethod kind(A this) => 1\nmethod kind(B this) => 2\nvar A a = na\nvar B b = na\nlist = array.new<B>(1)\nplot(a.kind() * 100 + b.kind() * 10 + list.get(0).kind())"
        ),
        122.0
    );
}

/// Built-ins against the reference manual's own Pine versions of them.
#[test]
fn reference_formulas() {
    const CASES: &[(&str, &str)] = &[
        (
            "sma",
            "f(x, y) =>\n    s = 0.0\n    for i = 0 to y - 1\n        s := s + x[i] / y\n    s\n|f(close, 15) - ta.sma(close, 15)",
        ),
        (
            "ema",
            "f(src, length) =>\n    alpha = 2 / (length + 1)\n    sum = 0.0\n    sum := na(sum[1]) ? ta.sma(src, length) : alpha * src + (1 - alpha) * nz(sum[1])\n|f(close, 15) - ta.ema(close, 15)",
        ),
        (
            "rma",
            "f(src, length) =>\n    alpha = 1 / length\n    sum = 0.0\n    sum := na(sum[1]) ? ta.sma(src, length) : alpha * src + (1 - alpha) * nz(sum[1])\n|f(close, 15) - ta.rma(close, 15)",
        ),
        (
            "wma",
            "f(x, y) =>\n    norm = 0.0\n    sum = 0.0\n    for i = 0 to y - 1\n        weight = (y - i) * y\n        norm := norm + weight\n        sum := sum + x[i] * weight\n    sum / norm\n|f(close, 15) - ta.wma(close, 15)",
        ),
        (
            "swma",
            "f(x) => x[3] * 1 / 6 + x[2] * 2 / 6 + x[1] * 2 / 6 + x[0] * 1 / 6\n|f(close) - ta.swma(close)",
        ),
        (
            "vwma",
            "f(x, y) => ta.sma(x * volume, y) / ta.sma(volume, y)\n|f(close, 15) - ta.vwma(close, 15)",
        ),
        (
            "hma",
            "f(src, length) => ta.wma(2 * ta.wma(src, length / 2) - ta.wma(src, length), math.floor(math.sqrt(length)))\n|f(close, 16) - ta.hma(close, 16)",
        ),
        (
            "alma",
            "f(series, windowsize, offset, sigma) =>\n    m = offset * (windowsize - 1)\n    s = windowsize / sigma\n    norm = 0.0\n    sum = 0.0\n    for i = 0 to windowsize - 1\n        weight = math.exp(-1 * math.pow(i - m, 2) / (2 * math.pow(s, 2)))\n        norm := norm + weight\n        sum := sum + series[windowsize - i - 1] * weight\n    sum / norm\n|f(close, 9, 0.85, 6) - ta.alma(close, 9, 0.85, 6)",
        ),
        (
            "rsi",
            "f(x, y) =>\n    u = math.max(x - x[1], 0)\n    d = math.max(x[1] - x, 0)\n    rs = ta.rma(u, y) / ta.rma(d, y)\n    100 - 100 / (1 + rs)\n|f(close, 7) - ta.rsi(close, 7)",
        ),
        (
            "stdev",
            "isZero(val, eps) => math.abs(val) <= eps\nSUM(fst, snd) =>\n    EPS = 1e-10\n    res = fst + snd\n    if isZero(res, EPS)\n        res := 0\n    else\n        if not isZero(res, 1e-4)\n            res := res\n        else\n            15\n    res\nf(src, length) =>\n    avg = ta.sma(src, length)\n    sumOfSquareDeviations = 0.0\n    for i = 0 to length - 1\n        sum = SUM(src[i], -avg)\n        sumOfSquareDeviations := sumOfSquareDeviations + sum * sum\n    math.sqrt(sumOfSquareDeviations / length)\n|f(close, 5) - ta.stdev(close, 5)",
        ),
        (
            "dev",
            "f(source, length) =>\n    mean = ta.sma(source, length)\n    sum = 0.0\n    for i = 0 to length - 1\n        val = source[i]\n        sum := sum + math.abs(val - mean)\n    sum / length\n|f(close, 10) - ta.dev(close, 10)",
        ),
        (
            "atr",
            "f(length) =>\n    trueRange = na(high[1]) ? high - low : math.max(math.max(high - low, math.abs(high - close[1])), math.abs(low - close[1]))\n    ta.rma(trueRange, length)\n|f(14) - ta.atr(14)",
        ),
        (
            "bb",
            "f(src, length, mult) =>\n    basis = ta.sma(src, length)\n    dev = mult * ta.stdev(src, length)\n    [basis, basis + dev, basis - dev]\n[a, b, c] = f(close, 5, 4)\n[d, e, g] = ta.bb(close, 5, 4)\n|b - e + (c - g)",
        ),
        (
            "kc",
            "f(src, length, mult, useTrueRange) =>\n    basis = ta.ema(src, length)\n    span = useTrueRange ? ta.tr : high - low\n    rangeEma = ta.ema(span, length)\n    [basis, basis + rangeEma * mult, basis - rangeEma * mult]\n[a, b, c] = f(close, 5, 4, true)\n[d, e, g] = ta.kc(close, 5, 4)\n[h, i, j] = f(close, 5, 4, false)\n[k, l, m] = ta.kc(close, 5, 4, false)\n|b - e + (c - g) + (i - l)",
        ),
        (
            "cmo",
            "f(src, len) =>\n    mom = ta.change(src)\n    sm1 = math.sum((mom >= 0) ? mom : 0.0, len)\n    sm2 = math.sum((mom >= 0) ? 0.0 : -mom, len)\n    100 * (sm1 - sm2) / (sm1 + sm2)\n|f(close, 5) - ta.cmo(close, 5)",
        ),
        (
            "cog",
            "f(source, length) =>\n    sum = math.sum(source, length)\n    num = 0.0\n    for i = 0 to length - 1\n        price = source[i]\n        num := num + price * (i + 1)\n    -num / sum\n|f(close, 10) - ta.cog(close, 10)",
        ),
        (
            "mfi",
            "f(src, length) =>\n    float upper = math.sum(volume * (ta.change(src) <= 0.0 ? 0.0 : src), length)\n    float lower = math.sum(volume * (ta.change(src) >= 0.0 ? 0.0 : src), length)\n    100.0 - (100.0 / (1.0 + upper / lower))\n|f(hlc3, 14) - ta.mfi(hlc3, 14)",
        ),
        (
            "tsi",
            "f(x, s, l) =>\n    pc = ta.change(x)\n    ta.ema(ta.ema(pc, l), s) / ta.ema(ta.ema(math.abs(pc), l), s)\n|f(close, 13, 25) - ta.tsi(close, 13, 25)",
        ),
        (
            "wpr",
            "f(length) =>\n    upper = ta.highest(length)\n    lower = ta.lowest(length)\n    100 * (close - upper) / (upper - lower)\n|f(14) - ta.wpr(14)",
        ),
        (
            "stoch",
            "|100 * (close - ta.lowest(low, 14)) / (ta.highest(high, 14) - ta.lowest(low, 14)) - ta.stoch(close, high, low, 14)",
        ),
        (
            "cci",
            "f(s, n) => (s - ta.sma(s, n)) / (0.015 * ta.dev(s, n))\n|f(hlc3, 20) - ta.cci(hlc3, 20)",
        ),
        ("obv", "|ta.cum(math.sign(ta.change(close)) * volume) - ta.obv"),
        ("pvt", "|ta.cum((ta.change(close) / close[1]) * volume) - ta.pvt"),
        (
            "wad",
            "f() =>\n    trueHigh = math.max(high, close[1])\n    trueLow = math.min(low, close[1])\n    mom = ta.change(close)\n    gain = (mom > 0) ? close - trueLow : (mom < 0) ? close - trueHigh : 0\n    ta.cum(gain)\n|f() - ta.wad",
        ),
        (
            "nvi",
            "f() =>\n    float ta_nvi = 1.0\n    float prevNvi = (nz(ta_nvi[1], 0.0) == 0.0) ? 1.0 : ta_nvi[1]\n    if nz(close, 0.0) == 0.0 or nz(close[1], 0.0) == 0.0\n        ta_nvi := prevNvi\n    else\n        ta_nvi := (volume < nz(volume[1], 0.0)) ? prevNvi + ((close - close[1]) / close[1]) * prevNvi : prevNvi\n    ta_nvi\n|f() - ta.nvi",
        ),
        (
            "supertrend",
            "f(factor, atrPeriod) =>\n    src = hl2\n    atr = ta.atr(atrPeriod)\n    upperBand = src + factor * atr\n    lowerBand = src - factor * atr\n    prevLowerBand = nz(lowerBand[1])\n    prevUpperBand = nz(upperBand[1])\n    lowerBand := lowerBand > prevLowerBand or close[1] < prevLowerBand ? lowerBand : prevLowerBand\n    upperBand := upperBand < prevUpperBand or close[1] > prevUpperBand ? upperBand : prevUpperBand\n    int _direction = na\n    float superTrend = na\n    prevSuperTrend = superTrend[1]\n    if na(atr[1])\n        _direction := 1\n    else if prevSuperTrend == prevUpperBand\n        _direction := close > upperBand ? -1 : 1\n    else\n        _direction := close < lowerBand ? 1 : -1\n    superTrend := _direction == -1 ? lowerBand : upperBand\n    [superTrend, _direction]\n[a, b] = f(3, 10)\n[c, d] = ta.supertrend(3, 10)\n|a - c + (b - d)",
        ),
        (
            "dmi",
            "f(diLength, smoothing) =>\n    up = ta.change(high)\n    down = -ta.change(low)\n    plusDM = na(up) ? na : (up > down and up > 0 ? up : 0)\n    minusDM = na(down) ? na : (down > up and down > 0 ? down : 0)\n    trur = ta.rma(ta.tr, diLength)\n    plus = fixnan(100 * ta.rma(plusDM, diLength) / trur)\n    minus = fixnan(100 * ta.rma(minusDM, diLength) / trur)\n    sum = plus + minus\n    adx = 100 * ta.rma(math.abs(plus - minus) / (sum == 0 ? 1 : sum), smoothing)\n    [plus, minus, adx]\n[a, b, c] = f(14, 14)\n[d, e, g] = ta.dmi(14, 14)\n|a - d + (b - e) + (c - g)",
        ),
        (
            "macd",
            "[a, b, c] = ta.macd(close, 12, 26, 9)\nm = ta.ema(close, 12) - ta.ema(close, 26)\n|a - m + (b - ta.ema(m, 9)) + (c - (m - ta.ema(m, 9)))",
        ),
        (
            "barssince",
            "f(c) =>\n    var int n = na\n    n := c ? 0 : n + 1\n    n\n|nz(f(close > open), -1) - nz(ta.barssince(close > open), -1)",
        ),
        (
            "valuewhen",
            "f(c, s) =>\n    var float a = na\n    var float b = na\n    if c\n        b := a\n        a := s\n    b\n|nz(f(close > open, high)) - nz(ta.valuewhen(close > open, high, 1))",
        ),
        (
            "cross",
            "m = ta.sma(close, 5)\n|(ta.crossover(close, m) ? 1 : 0) - (close > m and close[1] <= m[1] ? 1 : 0)",
        ),
        (
            "linreg",
            "f(src, len) =>\n    sx = 0.0\n    sy = 0.0\n    sxy = 0.0\n    sxx = 0.0\n    for i = 0 to len - 1\n        x = len - 1 - i\n        sx += x\n        sy += src[i]\n        sxy += x * src[i]\n        sxx += x * x\n    b = (len * sxy - sx * sy) / (len * sxx - sx * sx)\n    a = (sy - b * sx) / len\n    a + b * (len - 1 - 2)\n|f(close, 14) - ta.linreg(close, 14, 2)",
        ),
        (
            "correlation",
            "f(x, y, n) => (ta.sma(x * y, n) - ta.sma(x, n) * ta.sma(y, n)) / (ta.stdev(x, n) * ta.stdev(y, n))\n|f(close, volume, 10) - ta.correlation(close, volume, 10)",
        ),
        (
            "percentrank",
            "f(s, n) =>\n    c = 0.0\n    for i = 1 to n\n        c += s[i] <= s ? 1 : 0\n    c / n * 100\n|f(close, 10) - ta.percentrank(close, 10)",
        ),
        (
            "median",
            "f(s, n) =>\n    a = array.new<float>()\n    for i = 0 to n - 1\n        a.push(s[i])\n    a.median()\n|f(close, 8) - ta.median(close, 8)",
        ),
        (
            "mode",
            "|ta.mode(close > open ? 1 : 2, 5) - (math.sum(close > open ? 1 : 0, 5) >= 3 ? 1 : 2)",
        ),
        ("cum", "v = 0.0\nv := nz(v[1]) + volume\n|v - ta.cum(volume)"),
        (
            "sum",
            "f(s, n) =>\n    t = 0.0\n    for i = 0 to n - 1\n        t += s[i]\n    t\n|f(close, 7) - math.sum(close, 7)",
        ),
        (
            "rising",
            "|(ta.rising(close, 3) ? 1 : 0) - (close > close[1] and close > close[2] and close > close[3] ? 1 : 0)",
        ),
        (
            "vwap",
            "var float a = 0.0\nvar float b = 0.0\nif timeframe.change(\"D\") or bar_index == 0\n    a := 0.0\n    b := 0.0\na += hlc3 * volume\nb += volume\n|a / b - ta.vwap",
        ),
        (
            "sar",
            "f(start, inc, max) =>\n    var float result = na\n    var float maxMin = na\n    var float acceleration = na\n    var bool isBelow = false\n    bool isFirstTrendBar = false\n    if bar_index == 1\n        if close > close[1]\n            isBelow := true\n            maxMin := high\n            result := low[1]\n        else\n            isBelow := false\n            maxMin := low\n            result := high[1]\n        isFirstTrendBar := true\n        acceleration := start\n    result := result + acceleration * (maxMin - result)\n    if isBelow\n        if result > low\n            isFirstTrendBar := true\n            isBelow := false\n            result := math.max(high, maxMin)\n            maxMin := low\n            acceleration := start\n    else\n        if result < high\n            isFirstTrendBar := true\n            isBelow := true\n            result := math.min(low, maxMin)\n            maxMin := high\n            acceleration := start\n    if not isFirstTrendBar\n        if isBelow\n            if high > maxMin\n                maxMin := high\n                acceleration := math.min(acceleration + inc, max)\n        else\n            if low < maxMin\n                maxMin := low\n                acceleration := math.min(acceleration + inc, max)\n    if isBelow\n        result := math.min(result, low[1])\n        if bar_index > 1\n            result := math.min(result, low[2])\n    else\n        result := math.max(result, high[1])\n        if bar_index > 1\n            result := math.max(result, high[2])\n    result\n|f(0.02, 0.02, 0.2) - ta.sar(0.02, 0.02, 0.2)",
        ),
    ];
    let b = walk(&hours(700));
    let mut bad = Vec::new();
    for (name, case) in CASES {
        let (head, diff) = case.split_once('|').unwrap();
        for v in [5, 6] {
            let p = on(v, &format!("{head}plot({diff})"), &b, H1, Env::default());
            let n = p[0].iter().filter(|x| x.is_finite()).count();
            let worst = p[0]
                .iter()
                .filter(|x| x.is_finite())
                .fold(0.0f64, |m, x| m.max(x.abs()));
            if n < 500 || worst > 1e-5 {
                bad.push(format!("{name} (v{v}): {n} bars compared, worst {worst}"));
            }
        }
    }
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}

#[test]
fn extreme_values_are_errors_not_crashes() {
    for body in [
        "s = \"ab\"\nfor i = 0 to 60\n    s := s + s\nplot(str.length(s))",
        "s = \"a\"\nfor i = 0 to 60\n    s := str.replace_all(s, \"a\", \"aaaa\")\nplot(str.length(s))",
        "a = array.from(1, 2)\nfor i = 0 to 60\n    a := array.concat(a, a)\nplot(a.size())",
    ] {
        let mut st = load(6, body, Env::default()).study(&Inputs::default());
        st.run(&hours(2), H1);
        assert!(st.error().is_some(), "{body}");
    }
    let p = run(
        "big = 9223372036854775807\nplot(timestamp(99999, 1, 1))\nplot(timestamp(2024, big, 1))\nplot(timestamp(2024, 1, 1, big, 0))\nplot((-big - 1) % (bar_index - 1))\nplot(math.abs(-big - 1))\nplot(year(big))\nplot(ta.change(bar_index % 2 == 0 ? big : -big))\nplot(close[big])\nplot(big + bar_index)",
        4,
    );
    assert!(p[0][0].is_nan() && p[1][0].is_nan());
}

#[test]
fn drawdown_percent() {
    let src = "//@version=6\nstrategy(\"T\", initial_capital = 1000, default_qty_value = 1)\nif bar_index % 7 == 1\n    strategy.entry(\"L\", strategy.long)\nif bar_index % 7 == 5\n    strategy.close(\"L\")\nplot(strategy.max_drawdown_percent)\nplot(strategy.max_drawdown)";
    let mut st = compile(src, "t", Env::default()).unwrap().study(&Inputs::default());
    let o = st.run(&hours(60), H1);
    let (pct, money) = (o.plots[0][59], o.plots[1][59]);
    assert!(
        pct > 0.0 && money > 0.0 && (pct - money / 10.0).abs() < 0.05,
        "{pct} {money}"
    );
}

#[test]
#[ignore = "timing"]
fn timing_requests() {
    let b = walk(&nse_days(800));
    for (name, body) in [
        (
            "5 requests",
            "a = request.security(syminfo.tickerid, \"60\", ta.ema(close, 20))\nb = request.security(syminfo.tickerid, \"240\", ta.ema(close, 20))\nc = request.security(syminfo.tickerid, \"D\", ta.ema(close, 20))\nd = request.security(syminfo.tickerid, \"W\", ta.ema(close, 20))\ne = request.security(syminfo.tickerid, \"M\", ta.ema(close, 20))\nplot(a + b + c + d + e)",
        ),
        (
            "dates",
            "plot(dayofweek + hour + time_close % 7 + (session.islastbar ? 1 : 0))",
        ),
        (
            "changing lengths",
            "n = 10 + bar_index % 20\nplot(ta.sma(close, n) + ta.highest(n) + ta.stdev(close, n))",
        ),
    ] {
        let t = std::time::Instant::now();
        on(6, body, &b, M15, india());
        println!("{name}: {} bars in {:?}", b.len(), t.elapsed());
    }
}

/// Small pieces of the language, each with what it should give on the last
/// of 40 bars.
#[test]
fn the_language() {
    const CASES: &[(&str, f64)] = &[
        (
            "[a, b] = if close > 0\n    [1, 2]\nelse\n    [3, 4]\nplot(a * 10 + b)",
            12.0,
        ),
        (
            "[a, b] = switch\n    close < 0 => [1, 2]\n    => [3, 4]\nplot(a * 10 + b)",
            34.0,
        ),
        ("f(x, y = 2, z = 3) => x * 100 + y * 10 + z\nplot(f(1, z = 5))", 125.0),
        ("f(x) => x * 2\nplot(f(bar_index)[1])", 76.0),
        (
            "type P\n    int x = 0\np = P.new(bar_index)\nplot(bar_index > 2 ? p[1].x : 0)",
            38.0,
        ),
        (
            "type P\n    int x = 0\np = P.new(bar_index)\nplot(bar_index > 2 ? (p[2]).x : 0)",
            37.0,
        ),
        (
            "type P\n    int x = 0\nvar p = P.new(0)\np.x += 1\nq = p[1]\nplot(bar_index > 0 ? q.x : 0)",
            40.0,
        ),
        (
            "type Q\n    int v = 7\ntype P\n    Q q\np = P.new(Q.new())\np.q.v := 9\np.q.v += 1\nplot(p.q.v)",
            10.0,
        ),
        (
            "float[] a = array.new_float(3, 2.0)\nf(float[] x) => x.sum()\nplot(f(a))",
            6.0,
        ),
        (
            "array<float> a = array.from(3.0, 1.0, 2.0)\nb = a.copy()\nb.sort()\nplot(a.first() * 10 + b.first())",
            31.0,
        ),
        (
            "a = array.from(1, 2, 3)\nplot(a.get(-1) * 10 + a.slice(0, 2).sum())",
            33.0,
        ),
        (
            "s = \"a\"\ns += \"b\" + str.tostring(1)\nplot(s == \"ab1\" ? 1 : 0)",
            1.0,
        ),
        ("plot(not true or true ? 1 : 0)", 1.0),
        ("plot(1 < 2 == true ? 1 : 0)", 1.0),
        ("plot(-math.max(1, 2, 3) + 2 * -3 % 4)", -5.0),
        ("x = close > 1e9 ? 1 :\n     close > 1e8 ? 2 :\n     3\nplot(x)", 3.0),
        (
            "x = close > 0\n     and open > 0\n     and high > 0 // c\nplot(x ? 1 : 0)",
            1.0,
        ),
        (
            "f(x) =>\n\n    y = x + 1 // a\n\n    // b\n    y * 2\n\nplot(f(1))",
            4.0,
        ),
        ("f(x) =>\n\tif x > 1\n\t\tx * 2\n\telse\n\t\tx\nplot(f(3))", 6.0),
        (
            "n = 0\nfor i = 0 to 2\n    for j = 0 to 5\n        if j == 2\n            break\n        n += 1\nplot(n)",
            6.0,
        ),
        ("n = 0\nfor i = 10 to 0 by 3\n    n := n * 100 + i\nplot(n)", 10070401.0),
        (
            "m = map.new<string, int>()\nm.put(\"a\", 1)\nm.put(\"b\", 2)\nm.put(\"c\", 3)\nn = 0\nfor [k, v] in m\n    if k == \"b\"\n        continue\n    n += v\nplot(n)",
            4.0,
        ),
        (
            "n = 0\nwhile true\n    n += 1\n    if n > 4\n        break\nplot(n)",
            5.0,
        ),
        (
            "var a = array.new<int>()\nadd(x) => a.push(x)\nadd(1)\nadd(2)\nplot(a.size())",
            80.0,
        ),
        ("x = 1\nf() => x * 10\na = f()\nx := 2\nb = f()\nplot(a + b)", 30.0),
        ("x = 0\nx := nz(x[1]) + 1\nf() => x[1]\nplot(f())", 39.0),
        (
            "enum E\n    a = \"First\"\n    b\ne = E.b\nr = switch e\n    E.a => 1\n    E.b => 2\nplot(r)",
            2.0,
        ),
        (
            "enum E\n    a = \"First\"\n    b\nm = map.new<E, int>()\nm.put(E.a, 5)\nplot(m.get(E.a) + (str.tostring(E.a) == \"First\" ? 1 : 0))",
            6.0,
        ),
        (
            "plot(na(float(na)) and na(int(na)) and na(string(na)) == false ? 1 : 0)",
            1.0,
        ),
        (
            "line l = line(na)\nlabel b = label(na)\nplot(na(l) and na(b) ? 1 : 0)",
            1.0,
        ),
        ("plot(int(-5.9) * 10 + int(5.9))", -45.0),
        (
            "plot(math.round(2.5) * 100 + math.round(-2.5) * 10 + math.round(2.345, 2))",
            282.35,
        ),
        ("plot(5 / 2 + 7 % 3)", 3.5),
        ("plot(1 / 0 > 0 ? 1 : 0)", 0.0),
        ("plot(str.length(\"http://x.y\") + str.pos(\"abcabc\", \"c\"))", 12.0),
        (
            "plot(str.substring(\"hello\", 1, 3) == \"el\" and str.upper(\"a\") == \"A\" ? 1 : 0)",
            1.0,
        ),
        (
            "a = str.split(\"1,2,3\", \",\")\nt = 0.0\nfor x in a\n    t += str.tonumber(x)\nplot(t)",
            6.0,
        ),
        (
            "if bar_index > 0\n    var int k = 0\n    k += 1\n    label.new(bar_index, close, str.tostring(k))\nplot(1)",
            1.0,
        ),
        (
            "f() =>\n    var int k = 0\n    k += 1\n    k\na = f()\nb = bar_index % 2 == 0 ? f() : 0\nplot(a)",
            40.0,
        ),
        (
            "a = close > open ? 1 : close < open ? -1 : 0\nb = nz(a[1], 5)\nplot(b >= -1 ? 1 : 0)",
            1.0,
        ),
        (
            "x = array.new<float>(0)\nplot(na(x.max()) and na(x.avg()) ? 1 : 0)",
            1.0,
        ),
        (
            "m = matrix.new<float>(2, 3, 1.0)\nt = 0.0\nfor [i, row] in m\n    t += row.sum() * (i + 1)\nplot(t)",
            9.0,
        ),
        (
            "m = matrix.new<float>(2, 2, 0)\nm.set(0, 0, 2), m.set(1, 1, 4)\ni = m.inv()\nplot(i.get(0, 0) + i.get(1, 1) + m.det() + m.trace())",
            14.75,
        ),
        (
            "m = matrix.new<float>(2, 2, 1)\nm.set(0, 0, 2), m.set(1, 1, 2)\ne = m.eigenvalues()\nplot(e.get(0) * 10 + e.get(1))",
            31.0,
        ),
        (
            "m = matrix.new<float>(2, 3, 1)\nplot(m.rank() + m.pinv().rows() + m.kron(m).columns())",
            13.0,
        ),
        ("import TradingView/ta/9 as t\nplot(na(t.ema2(close, 5)) ? 0 : 1)", 1.0),
        (
            "a = request.security(syminfo.tickerid, timeframe.period, close)\nplot(a - close)",
            0.0,
        ),
        (
            "[o, c] = request.security(syminfo.tickerid, \"\", [open, close])\nplot(o - open + c - close)",
            0.0,
        ),
        (
            "type R\n    float a\n    float b\nf() => R.new(close, open)\nr = request.security(syminfo.tickerid, \"60\", f())\nplot(r.a - close)",
            0.0,
        ),
        (
            "plot(ta.highest(5) - ta.highest(high, 5) + ta.lowest(5) - ta.lowest(low, 5))",
            0.0,
        ),
        (
            "plot(timeframe.in_seconds(\"60\") / 3600 + timeframe.in_seconds(\"D\") / 86400)",
            2.0,
        ),
        ("plot(color.t(color.new(color.red, 70)) + color.r(#FF000080))", 325.0),
        ("a = 3, b = 4\nc = a + b, d = c * 2\nplot(d)", 14.0),
        ("f(x) => [x, x * 2]\n[a, _] = f(3)\n[_, d] = f(4)\nplot(a + d)", 11.0),
        (
            "f(simple int n, series float s = close) => ta.sma(s, n)\nplot(f(3) - ta.sma(close, 3))",
            0.0,
        ),
        ("export_me = 1\nimport_me = 2\nto = 3\nby = 4\nplot(to + by)", 7.0),
        (
            "g(x) => x + 1\nf(x) => g(x) * 2\nh(x) =>\n    a = f(x)\n    b = f(a)\n    a + b\nplot(h(1))",
            14.0,
        ),
        (
            "var float x = na\nif na(x)\n    x := close\nelse\n    x := math.max(x, close)\nplot(x - ta.max(close))",
            0.0,
        ),
        ("b = bool(1)\nplot(b ? 1 : 0)", 1.0),
        ("x = if false\n    5\nplot(na(x) ? 1 : 0)", 1.0),
        ("plot(dayofweek == dayofweek.tuesday ? 1 : 0)", 1.0),
        ("plot(str.tonumber(str.format_time(time, \"HH\", \"UTC\")))", 15.0),
        ("t = time(\"D\")\nplot(hour(t) + (time - t) / 3600000)", 15.0),
        (
            "a = array.new<float>(5, 1)\na.fill(2, 1, 3)\nplot(a.sum() + a.indexof(2) + a.lastindexof(2))",
            10.0,
        ),
        (
            "a = array.from(5, 3, 8)\ni = a.sort_indices()\nplot(i.get(0) * 100 + i.get(1) * 10 + i.get(2))",
            102.0,
        ),
        (
            "a = array.from(1, 3, 5, 7)\nplot(a.binary_search(5) * 100 + a.binary_search_leftmost(4) * 10 + a.binary_search_rightmost(4))",
            212.0,
        ),
        (
            "a = array.from(1, 2, 3, 4)\nplot(a.percentile_linear_interpolation(50) + a.median() + a.stdev() - a.variance())",
            2.5 + 2.5 + 1.118033988749895 - 1.25,
        ),
        ("plot(math.sum(1, 5) + ta.cum(1))", 45.0),
        (
            "x = ta.change(close) > 0\nplot(ta.barssince(true) + (x or not x ? 1 : 0))",
            1.0,
        ),
        (
            "f(x, n) =>\n    s = 0.0\n    for i = 0 to n - 1\n        s += x[i]\n    s / n\nplot(f(close, 4) - ta.sma(close, 4))",
            0.0,
        ),
        (
            "n = bar_index % 3\nplot(close[n] - (n == 0 ? close : n == 1 ? close[1] : close[2]))",
            0.0,
        ),
        ("f(src) => src[1] + src[2]\nplot(f(close) - close[1] - close[2])", 0.0),
        ("f(src) => ta.sma(src, 3)[1]\nplot(f(hl2) - ta.sma(hl2, 3)[1])", 0.0),
        ("a = close, b = a[1], c = b[1]\nplot(c - close[2])", 0.0),
    ];
    let mut bad = Vec::new();
    for (src, want) in CASES {
        let got = std::panic::catch_unwind(|| last(src));
        match got {
            Ok(x) if eq(x, *want) => {}
            Ok(x) => bad.push(format!("{src}\n  gave {x}, not {want}")),
            Err(e) => bad.push(format!(
                "{src}\n  {}",
                e.downcast_ref::<String>().cloned().unwrap_or_default()
            )),
        }
    }
    assert!(bad.is_empty(), "{} wrong:\n{}", bad.len(), bad.join("\n\n"));
}

#[test]
fn heikin_ashi_requests() {
    let p = run(
        "ho = float(na)\nhc = ohlc4\nho := na(ho[1]) ? (open + close) / 2 : (ho[1] + hc[1]) / 2\n\
         [a, b, c] = request.security(ticker.heikinashi(syminfo.tickerid), timeframe.period, [open, close, high])\n\
         plot(a - ho)\nplot(b - hc)\nplot(c - math.max(high, ho, hc))\n\
         [d, e] = request.security(syminfo.tickerid, \"240\", [open, close])\nplot(request.security(ticker.heikinashi(syminfo.tickerid), \"240\", close))\nplot(request.security(syminfo.tickerid, \"240\", ohlc4))\n\
         plot(request.security(ticker.standard(ticker.heikinashi(syminfo.tickerid)), timeframe.period, close) - close)",
        80,
    );
    for k in [0, 1, 2, 5] {
        assert!(p[k].iter().all(|x| x.abs() < 1e-9), "plot {k}");
    }
    same(&p[3], &p[4]);
}

#[test]
fn display_says_where_a_plot_shows() {
    let s = load(
        6,
        "show = input.bool(false)\nplot(close, display = display.data_window)\nplot(close, display = display.none)\nplot(close, display = display.all - display.status_line)\nplot(close, display = display.pane + display.price_scale)\nplot(close, display = display.status_line + display.data_window)\nplot(close, display = show ? display.all : display.none)\nplot(close, display = display.all)\nplot(close)",
        Env::default(),
    );
    let shown: Vec<bool> = s.meta().plots.iter().map(|p| p.visible).collect();
    assert_eq!(shown, [false, false, true, true, false, false, true, true]);
}

#[test]
fn extended_hours_stay_out_of_the_day() {
    // 04:00 to 20:00 New York, hourly, two days; 09:30 to 16:00 is the day.
    let mut b = Vec::new();
    for d in ["2024-01-02", "2024-01-03"] {
        let t = ms(&format!("{d}T09:30:00Z"));
        for k in 0..16 {
            b.push(bar(t + k * 3_600_000, 100.0 + b.len() as f64));
        }
    }
    let p = on(
        6,
        "[o, c, h, v] = request.security(syminfo.tickerid, \"D\", [open, close, high, volume])\nplot(o)\nplot(c)\nplot(h)\nplot(v)\nplot(request.security(syminfo.tickerid, \"D\", close[1]))",
        &b,
        H1,
        us(),
    );
    // Bars open at 04:30, 05:30, …: 09:30 is the 6th, 15:30 the 12th.
    let day = |k: usize| &b[k * 16 + 5..k * 16 + 12];
    for at in [11, 12, 15, 20] {
        assert_eq!(p[0][at], day(0)[0].open, "bar {at}");
        assert_eq!(p[1][at], day(0)[6].close, "bar {at}");
        assert_eq!(p[2][at], day(0)[6].high, "bar {at}");
        assert_eq!(p[3][at], 70.0, "bar {at}");
    }
    assert_eq!(p[1][31], day(1)[6].close);
    assert_eq!(p[4][31], day(0)[6].close, "one daily bar a day");
}

#[test]
fn slices_share_their_array() {
    assert_eq!(
        last(
            "a = array.from(5, 3, 8, 1)\nb = a.slice(1, 4)\nb.set(0, 9)\nc = b.copy()\nc.set(1, 0)\nt = 0\nfor x in b\n    t += x\nplot(a.get(1) * 1000 + t * 10 + b.size())"
        ),
        9183.0
    );
    assert_eq!(
        last(
            "a = array.from(5, 3, 8, 1)\narray.slice(a, 1, 4).sort()\nplot(a.get(1) * 100 + a.get(2) * 10 + a.get(3))"
        ),
        138.0
    );
    assert_eq!(
        text(6, "out = str.tostring(array.slice(array.from(1, 2, 3), 1, 3))"),
        "[2, 3]"
    );
}

#[test]
fn varip_survives_ticks() {
    let s = load(
        6,
        "varip int ticks = 0\nticks += 1\nvar int bars = 0\nbars += 1\nf() =>\n    varip int n = 0\n    n += 1\n    n\nplot(ticks)\nplot(bars)\nplot(f())",
        Env::default(),
    );
    let mut inst = opentern_series::study::Instance::with_factory(s.meta(), &Inputs::default(), s.factory());
    let b = hours(6);
    inst.recompute(&b[..4], &[true; 4], H1);
    let at = |i: &opentern_series::study::Instance, k: usize| [i.out.plots[0][k], i.out.plots[1][k], i.out.plots[2][k]];
    assert_eq!(at(&inst, 3), [4.0, 4.0, 4.0]);
    for _ in 0..2 {
        inst.update(&b[..4], &[true; 4], H1);
    }
    assert_eq!(at(&inst, 3), [6.0, 4.0, 6.0]);
    // Bar 3 closes (one more run), and bar 4 starts.
    inst.update(&b[..5], &[true; 5], H1);
    assert_eq!(at(&inst, 3), [7.0, 4.0, 7.0]);
    assert_eq!(at(&inst, 4), [8.0, 5.0, 8.0]);
    // Starting over forgets it.
    inst.recompute(&b[..5], &[true; 5], H1);
    assert_eq!(at(&inst, 4), [5.0, 5.0, 5.0]);
}

#[test]
fn slices_of_slices() {
    assert_eq!(
        last(
            "a = array.from(1, 2, 3, 4, 5)\nb = a.slice(1, 5)\nfor i = 0 to 50000\n    b := b.slice(0, b.size())\nc = b.slice(1, 3)\nplot(c.get(0) * 10 + c.get(1))"
        ),
        34.0
    );
}

#[test]
fn version_4_scripts() {
    let src = "//@version=4\nstudy(title=\"Old\", shorttitle=\"O\", overlay=true, resolution=\"\")\n\
        len = input(14, minval=1, title=\"Length\")\nsrc = input(close, title=\"Source\", type=input.source)\n\
        show = input(true, type=input.bool)\nk = input(2.0, type=input.float, step=0.5)\nm = input(\"SMA\", options=[\"SMA\", \"EMA\"])\n\
        tf = input(\"D\", type=input.resolution)\n\
        ma = m == \"SMA\" ? sma(src, len) : ema(src, len)\nup = ma + k * stdev(src, len)\nr = rsi(src, len)\n\
        h = highest(len), l = lowest(low, len)\nx = iff(close > ma, 1, 0)\nc = crossover(close, ma)\n\
        d = security(syminfo.tickerid, tf, close[1], lookahead=barmerge.lookahead_on)\n\
        p1 = plot(show ? ma : na, color=color.red, transp=50), p2 = plot(up)\nfill(p1, p2, transp=90)\n\
        plotshape(c, style=shape.triangleup, location=location.belowbar, text=tostring(round(r)))\n\
        plot(abs(close - max(open, close)) + sum(volume, 3) + change(close) + valuewhen(c, close, 0) + barssince(c) + tr + atr(5) + x + d + pow(2, 3) + h - l)\n\
        [a, b, e] = macd(close, 12, 26, 9)\nplot(a), plot(linreg(close, len, 0)), plot(offset(close, 2) - close[2])\n\
        if barstate.islast\n    label.new(bar_index, high, tostring(close, \"#.##\"))";
    let mut st = compile(src, "t", Env::default())
        .unwrap_or_else(|e| panic!("{e}"))
        .study(&Inputs::default());
    let out = st.run(&walk(&hours(200)), H1);
    assert!(st.error().is_none(), "{:?}", st.error());
    assert!(out.plots.iter().filter(|p| p[199].is_finite()).count() >= 5);
    assert_eq!(out.plots.last().unwrap()[100], 0.0);
    // A v5 script that forgets the namespace is told which.
    let e = compile(
        "//@version=5\nindicator(\"x\")\nplot(sma(close, 5))",
        "t",
        Env::default(),
    )
    .err()
    .unwrap();
    assert!(e.message.contains("ta.sma"), "{e}");
    // v3's bare colours, and no version at all is v1 to v3 territory too.
    let s = compile(
        "//@version=3\nstrategy(\"x\")\nplot(close, color=red)\nif crossover(close, sma(close, 5))\n    strategy.entry(\"L\", true)",
        "t",
        Env::default(),
    );
    assert!(s.is_ok(), "{}", s.err().unwrap());
}

/// Trades and fills of a strategy on given bars (open, high, low, close).
fn fills(head: &str, body: &str, b: &[(f64, f64, f64, f64)]) -> opentern_engine::Broker {
    let bars: Vec<Bar> = b
        .iter()
        .enumerate()
        .map(|(i, x)| Bar {
            time: ms("2024-01-01T00:00:00Z") + i as i64 * 3_600_000,
            open: x.0,
            high: x.1,
            low: x.2,
            close: x.3,
            volume: 1.0,
        })
        .collect();
    let src = format!("//@version=6\nstrategy(\"T\", initial_capital = 100000, default_qty_value = 1{head})\n{body}");
    let mut st = compile(&src, "t", Env::default())
        .unwrap_or_else(|e| panic!("{e}\n{src}"))
        .study(&Inputs::default());
    st.run(&bars, H1);
    assert!(st.error().is_none(), "{:?}", st.error());
    st.broker().unwrap().clone()
}

const FLAT: (f64, f64, f64, f64) = (100.0, 101.0, 99.0, 100.0);

#[test]
fn entries_close_oldest_first_unless_any() {
    let body = "if bar_index == 0\n    strategy.entry(\"A\", strategy.long)\nif bar_index == 1\n    strategy.entry(\"B\", strategy.long)\nif bar_index == 3\n    strategy.close(\"B\")";
    let b = fills(", pyramiding = 2", body, &[FLAT; 6]);
    assert_eq!(
        (b.trades.len(), b.trades[0].entry_id.as_str(), b.lots[0].id.as_str()),
        (1, "A", "B")
    );
    let b = fills(", pyramiding = 2, close_entries_rule = \"ANY\"", body, &[FLAT; 6]);
    assert_eq!((b.trades[0].entry_id.as_str(), b.lots[0].id.as_str()), ("B", "A"));
}

#[test]
fn limit_orders_have_no_slippage() {
    let b = fills(
        ", slippage = 10",
        "if bar_index == 0\n    strategy.entry(\"L\", strategy.long, limit = 99.5)\nif bar_index == 2\n    strategy.entry(\"S\", strategy.short)",
        &[FLAT; 5],
    );
    assert_eq!(b.trades[0].entry_price, 99.5);
    assert!(
        eq(b.trades[0].exit_price, 99.9),
        "a market order has it: {}",
        b.trades[0].exit_price
    );
}

#[test]
fn a_stop_limit_waits_for_its_limit() {
    let body = "if bar_index == 0\n    strategy.entry(\"L\", strategy.long, limit = 99, stop = 102)";
    // 102 trades, then the price stays above 99: nothing.
    let b = fills(
        "",
        body,
        &[FLAT, (100.0, 103.0, 100.0, 103.0), (103.0, 104.0, 101.0, 103.0)],
    );
    assert!(b.lots.is_empty(), "{:?}", b.lots);
    // It comes back to 99 later.
    let b = fills(
        "",
        body,
        &[FLAT, (100.0, 103.0, 100.0, 103.0), (103.0, 104.0, 98.0, 100.0)],
    );
    assert_eq!(b.lots[0].price, 99.0);
    // Below 99 before 102 is ever reached: the stop hasn't armed it.
    let b = fills("", body, &[FLAT, (100.0, 101.0, 95.0, 96.0), (96.0, 97.0, 95.0, 96.0)]);
    assert!(b.lots.is_empty());
    // A limit above the stop fills at the stop.
    let b = fills(
        "",
        "if bar_index == 0\n    strategy.entry(\"L\", strategy.long, limit = 105, stop = 102)",
        &[FLAT, (100.0, 103.0, 100.0, 103.0)],
    );
    assert_eq!(b.lots[0].price, 102.0);
}

#[test]
fn limits_can_need_to_be_exceeded() {
    let body = "if bar_index == 0\n    strategy.entry(\"L\", strategy.long, limit = 99)";
    assert_eq!(fills("", body, &[FLAT; 4]).lots.len(), 1);
    assert_eq!(
        fills(", backtest_fill_limits_assumption = 5", body, &[FLAT; 4])
            .lots
            .len(),
        0
    );
    let deeper = [FLAT, (100.0, 101.0, 98.9, 100.0)];
    assert_eq!(
        fills(", backtest_fill_limits_assumption = 5", body, &deeper).lots[0].price,
        99.0
    );
    // Exits too.
    let exit = "if bar_index == 0\n    strategy.entry(\"L\", strategy.long)\nif bar_index == 1\n    strategy.exit(\"X\", \"L\", limit = 101)";
    assert_eq!(fills("", exit, &[FLAT; 5]).trades.len(), 1);
    assert_eq!(
        fills(", backtest_fill_limits_assumption = 5", exit, &[FLAT; 5])
            .trades
            .len(),
        0
    );
}

#[test]
fn background_offsets_to_the_left() {
    let s = load(
        6,
        "bgcolor(bar_index == 5 ? color.red : na, offset = -2)\nbarcolor(bar_index == 6 ? color.red : na, offset = -1)",
        Env::default(),
    );
    let out = s.study(&Inputs::default()).run(&hours(9), H1);
    assert_eq!(
        (
            out.bg.iter().position(|c| *c != 0),
            out.bar.iter().position(|c| *c != 0)
        ),
        (Some(3), Some(5))
    );
    let b = hours(9);
    let mut inst = opentern_series::study::Instance::with_factory(s.meta(), &Inputs::default(), s.factory());
    inst.recompute(&b, &[true; 9], H1);
    assert_eq!((inst.out.bg, inst.out.bar), (out.bg, out.bar));
}

#[test]
fn what_is_not_simulated_is_said() {
    let src = "//@version=6\nstrategy(\"T\", calc_on_order_fills = true)\nplot(close)";
    let st = compile(src, "t", Env::default()).unwrap().study(&Inputs::default());
    assert!(st.logs()[0].contains("calc_on_order_fills"));
}

#[test]
fn a_stop_limit_placed_again_stays_armed() {
    let b = fills(
        "",
        "if strategy.position_size == 0\n    strategy.entry(\"L\", strategy.long, limit = 99, stop = 102)",
        &[
            FLAT,
            (100.0, 103.0, 100.0, 103.0),
            (103.0, 104.0, 100.5, 101.0),
            (101.0, 101.5, 98.0, 99.0),
        ],
    );
    assert_eq!(b.lots.first().map(|l| (l.bar, l.price)), Some((3, 99.0)));
}

/// What becomes of every script in `PINE_DIR` (many, other people's): one
/// line each, errors first.
#[test]
#[ignore = "needs PINE_DIR"]
fn survey() {
    let dir = std::env::var("PINE_DIR").unwrap();
    let mut files: Vec<_> = std::fs::read_dir(dir).unwrap().flatten().map(|e| e.path()).collect();
    files.sort();
    let (b, tf, env) = match std::env::var("PINE_ON").as_deref() {
        Ok("us") => (walk(&weekdays("2021-01-01T00:00:00Z", 800)), D1, us()),
        Ok("nse") => (walk(&nse_days(32)), M15, india()),
        _ => (walk(&hours(800)), H1, Env::default()),
    };
    for f in files {
        let src = std::fs::read_to_string(&f).unwrap();
        let name = f.file_name().unwrap().to_string_lossy().into_owned();
        let (b2, env, t) = (b.clone(), env.clone(), std::time::Instant::now());
        let r = std::thread::Builder::new()
            .stack_size(16 << 20)
            .spawn(move || match compile(&src, "t", env) {
                Err(e) => format!("COMPILE {e}"),
                Ok(s) if s.kind() == crate::Kind::Library => "LIBRARY".into(),
                Ok(s) => {
                    let mut st = s.study(&Inputs::default());
                    let out = st.run(&b2, tf);
                    match st.error() {
                        Some(e) => format!("RUNTIME {e}"),
                        None => match live_differs(&s, &b2, tf, &out) {
                            Some(e) => format!("LIVE {e}"),
                            None => "ok".into(),
                        },
                    }
                }
            })
            .unwrap()
            .join()
            .unwrap_or("PANIC".into());
        println!("SURVEY\t{name}\t{:.1}s\t{r}", t.elapsed().as_secs_f64());
    }
}

#[test]
fn from_public_scripts() {
    assert_eq!(
        last("var G = \"Main\"\nvar int K = 3\nn = input.int(K, group = G)\nhline(K)\nplot(n + K)"),
        6.0
    );
    assert_eq!(
        last("x = high[na(close) ? 0 : int(na)]\nplot(na(x) and na((close + 1)[int(na)]) ? 1 : 0)"),
        1.0
    );
    assert_eq!(
        last("f(a, b, c) =>\n    d = a\n    d < b or c > a\nplot(f(1, 2, 0) ? 1 : 0)"),
        1.0
    );
    assert_eq!(
        last(
            "a = array.from(1, 2, 3, 4)\nb = a.slice(1, 3)\nx = b.pop()\nb.insert(0, 9)\nb.push(8)\nplot(a.get(1) * 1000 + a.get(2) * 100 + a.get(3) * 10 + a.size() + b.size() / 10 + x / 100)"
        ),
        9285.33
    );
    assert_eq!(
        last("import TradingView/ta/7 as t\nplot(na(t.ema2(close, 5)) ? 0 : 1)"),
        1.0
    );
    assert_eq!(
        last("plot(9223372036854776000 > 1 ? 1 : 0)\nx = plotcandle(open, high, low, close)"),
        1.0
    );
    for src in [
        "//@version=4\nstudy(\"x\")\nswitch = 0\nswitch := nz(switch[1]) + 1\na = 2 or switch\nvar = 5\nplot(var + switch + input(2, group=\"g\", minval=1), title=tostring(x=1, y=\"#\"))",
        "//@version=3\nstudy(\"x\")\nplot(close, style=line)\nhline(0, linestyle=line)",
        "//@version=6\nindicator(\"x\")\nplot(request.security(ticker.modify(syminfo.tickerid, backadjustment = backadjustment.on), \"D\", close))",
    ] {
        compile(src, "t", Env::default()).unwrap_or_else(|e| panic!("{e}\n{src}"));
    }
}

#[test]
fn history_ends_once() {
    let s = load(
        6,
        "plot(barstate.islastconfirmedhistory ? 1 : 0)\nplot(barstate.isrealtime ? 1 : 0)\nplot(barstate.ishistory ? 1 : 0)",
        Env::default(),
    );
    let b = hours(8);
    let mut inst = opentern_series::study::Instance::with_factory(s.meta(), &Inputs::default(), s.factory());
    inst.recompute(&b[..5], &[true; 5], H1);
    for k in 6..=8 {
        inst.update(&b[..k], &[true; 8], H1);
    }
    assert_eq!(inst.out.plots[0], [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0]);
    assert_eq!(inst.out.plots[1], [0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0]);
    assert_eq!(inst.out.plots[2], [1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0]);
    // All closed: the last one.
    assert_eq!(s.study(&Inputs::default()).run(&b, H1).plots[0][7], 1.0);
}

#[test]
fn what_the_manual_says() {
    // Strings: "#.########" by default; "#,###.###" in str.format.
    assert_eq!(
        text(
            6,
            "out = str.tostring(1 / 3) + \" \" + str.tostring(0.5) + \" \" + str.format(\"{0}\", 0.5)"
        ),
        "0.33333333 0.5 0.5"
    );
    assert_eq!(
        text(
            6,
            "out = str.tostring(0.25, \"#.#%\") + \" \" + str.format(\"{0,number,percent}\", 0.256)"
        ),
        "25% 26%"
    );
    // Labels: white text since v6.
    for (v, c) in [(5, 0x363A_45FF), (6, 0xFFFF_FFFF_u32)] {
        let mut st = load(v, "label.new(bar_index, close, \"x\")", Env::default()).study(&Inputs::default());
        st.run(&hours(2), H1);
        assert_eq!(st.drawings().labels[0].textcolor, c);
    }
    // Loops count down, and follow a collection's size.
    assert_eq!(last("n = 0\nfor i = 0 to -1\n    n += 1\nplot(n)"), 2.0);
    assert_eq!(
        last("a = array.from(1)\nn = 0\nfor x in a\n    n += 1\n    if n < 4\n        a.push(1)\nplot(n)"),
        4.0
    );
}

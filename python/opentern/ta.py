"""Technical analysis helpers with Pine's semantics (same rules as
Opentern's built-ins: EMA and RMA seed with the SMA, ``nan`` propagates,
windows need ``length`` values). Pure Python; series in, list out."""

import math

nan = float("nan")


def _l(x):
    if hasattr(x, "tolist"):
        x = x.tolist()
    return [nan if v is None else float(v) for v in x]


def _isnan(v):
    return v != v


def sma(src, length):
    x = _l(src)
    out = []
    for i in range(len(x)):
        w = x[i - length + 1 : i + 1] if i + 1 >= length else []
        out.append(sum(w) / length if w and not any(_isnan(v) for v in w) else nan)
    return out


def _smooth(src, length, alpha):
    x = _l(src)
    seed = sma(x, length)
    out, prev = [], nan
    for i, v in enumerate(x):
        prev = seed[i] if _isnan(prev) else alpha * v + (1 - alpha) * prev
        out.append(prev)
    return out


def ema(src, length):
    return _smooth(src, length, 2.0 / (length + 1))


def rma(src, length):
    return _smooth(src, length, 1.0 / length)


def wma(src, length):
    x = _l(src)
    norm = length * (length + 1) / 2
    out = []
    for i in range(len(x)):
        if i + 1 < length:
            out.append(nan)
            continue
        w = x[i - length + 1 : i + 1]
        out.append(sum(v * (k + 1) for k, v in enumerate(w)) / norm)
    return out


def vwma(src, volume, length):
    x, v = _l(src), _l(volume)
    a = sma([p * q for p, q in zip(x, v)], length)
    b = sma(v, length)
    return [p / q if q else nan for p, q in zip(a, b)]


def stdev(src, length):
    x = _l(src)
    m = sma(x, length)
    out = []
    for i in range(len(x)):
        if _isnan(m[i]):
            out.append(nan)
            continue
        ss = 0.0
        for v in x[i - length + 1 : i + 1]:
            d = v - m[i]
            ss += 0.0 if abs(d) <= 1e-10 else d * d
        out.append(math.sqrt(ss / length))
    return out


def _window(src, length, pick):
    x = _l(src)
    out = []
    for i in range(len(x)):
        if i + 1 < length:
            out.append(nan)
            continue
        w = [v for v in x[i - length + 1 : i + 1] if not _isnan(v)]
        out.append(pick(w) if w else nan)
    return out


def highest(src, length):
    return _window(src, length, max)


def lowest(src, length):
    return _window(src, length, min)


def change(src, length=1):
    x = _l(src)
    return [x[i] - x[i - length] if i >= length else nan for i in range(len(x))]


def rsi(src, length):
    x = _l(src)
    up = [nan] + [max(x[i] - x[i - 1], 0.0) for i in range(1, len(x))]
    dn = [nan] + [max(x[i - 1] - x[i], 0.0) for i in range(1, len(x))]
    u, d = rma(up, length), rma(dn, length)
    out = []
    for a, b in zip(u, d):
        if _isnan(a) or _isnan(b):
            out.append(nan)
        elif b == 0:
            out.append(100.0)
        elif a == 0:
            out.append(0.0)
        else:
            out.append(100 - 100 / (1 + a / b))
    return out


def tr(bars, handle_na=True):
    h, l, c = _l(bars.high), _l(bars.low), _l(bars.close)
    out = []
    for i in range(len(c)):
        if i == 0:
            out.append(h[0] - l[0] if handle_na else nan)
        else:
            out.append(max(h[i] - l[i], abs(h[i] - c[i - 1]), abs(l[i] - c[i - 1])))
    return out


def atr(bars, length):
    return rma(tr(bars, True), length)


def macd(src, fast=12, slow=26, signal=9):
    f, s = ema(src, fast), ema(src, slow)
    m = [a - b for a, b in zip(f, s)]
    sig = ema(m, signal)
    return m, sig, [a - b for a, b in zip(m, sig)]


def bb(src, length=20, mult=2.0):
    basis = sma(src, length)
    dev = stdev(src, length)
    return basis, [b + mult * d for b, d in zip(basis, dev)], [b - mult * d for b, d in zip(basis, dev)]


def crossover(a, b):
    a, b = _series2(a, b)
    return [i > 0 and a[i] > b[i] and a[i - 1] <= b[i - 1] for i in range(len(a))]


def crossunder(a, b):
    a, b = _series2(a, b)
    return [i > 0 and a[i] < b[i] and a[i - 1] >= b[i - 1] for i in range(len(a))]


def _series2(a, b):
    if isinstance(a, (int, float)) and isinstance(b, (int, float)):
        return [float(a)], [float(b)]
    n = len(b) if isinstance(a, (int, float)) else len(_l(a))
    a = [float(a)] * n if isinstance(a, (int, float)) else _l(a)
    b = [float(b)] * n if isinstance(b, (int, float)) else _l(b)
    return a, b


def cum(src):
    out, t = [], 0.0
    for v in _l(src):
        t += 0.0 if _isnan(v) else v
        out.append(t)
    return out

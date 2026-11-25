"""Opentern's Python scripting SDK.

A script declares itself, its inputs, and a ``compute(bars)`` function that
plots (and, for strategies, places orders). Opentern runs it in its own
process whenever the chart's data changes.

    from opentern import *

    indicator("SMA cross", overlay=True)
    fast = input.int(9, "Fast length")
    slow = input.int(21, "Slow length")

    def compute(bars):
        plot(ta.sma(bars.close, fast), "Fast", color="#2962FF")
        plot(ta.sma(bars.close, slow), "Slow", color="#FF6D00")

Series are plain lists of floats (``nan`` for no value); numpy arrays and
pandas Series are accepted anywhere a series is.
"""

import math

from . import ta  # noqa: F401  (re-exported)

nan = float("nan")
na = nan

__all__ = [
    "indicator",
    "strategy",
    "input",
    "plot",
    "hline",
    "ta",
    "nan",
    "na",
    "is_na",
    "Bars",
]


def is_na(x):
    try:
        return x is None or bool(x != x)
    except (TypeError, ValueError):  # pandas.NA has no truth value
        return True


def _num(x):
    """A finite number, or None for no value."""
    if is_na(x):
        return None
    x = float(x)
    return x if math.isfinite(x) else None


class _State:
    def reset(self, inputs):
        self.values = dict(inputs)
        self.meta_ = {"name": "Script", "short": "", "overlay": False, "kind": "indicator", "props": {}}
        self.inputs = []
        self.plots = []
        self.hlines = []
        self.intents = []
        self._seen = set()

    def meta(self):
        m = dict(self.meta_)
        m["inputs"] = self.inputs
        return m

    def output(self, n):
        return {
            "plots": [
                dict(p, values=[_num(v) for v in _series(p["values"], n)]) for p in self.plots
            ],
            "hlines": self.hlines,
            "intents": self.intents,
        }


_state = _State()
_state.reset({})


def _series(x, n):
    """Any series-like value as a list of n floats (left-padded with nan)."""
    if hasattr(x, "tolist"):
        x = x.tolist()
    if isinstance(x, (int, float)) or x is None:
        return [nan if x is None else float(x)] * n
    x = [nan if is_na(v) else float(v) for v in x]
    if len(x) < n:
        x = [nan] * (n - len(x)) + x
    return x[-n:] if n else []


def _bools(x, n):
    if hasattr(x, "tolist"):
        x = x.tolist()
    if isinstance(x, (bool, int, float)) or x is None:
        return [not is_na(x) and bool(x)] * n
    x = [not is_na(v) and bool(v) for v in x]
    if len(x) < n:
        x = [False] * (n - len(x)) + x
    return x[-n:] if n else []


def indicator(title, shorttitle=None, overlay=False, precision=None):
    _state.meta_.update(name=title, short=shorttitle or title, overlay=bool(overlay), kind="indicator")
    if precision is not None:
        _state.meta_["precision"] = int(precision)


def _slug(title):
    s = "".join(c.lower() if c.isalnum() else "_" for c in title).strip("_")
    return s or "input"


class _Input:
    def _add(self, kind, default, title, **extra):
        title = title or f"Input {len(_state.inputs) + 1}"
        key = _slug(title)
        while key in _state._seen:
            key += "_"
        _state._seen.add(key)
        _state.inputs.append(dict(id=key, title=title, type=kind, default=default, **extra))
        value = _state.values.get(key, default)
        if kind == "int":
            return int(value)
        if kind == "float":
            return float(value)
        if kind == "bool":
            return bool(value)
        return value

    def int(self, defval, title=None, minval=None, maxval=None, step=1):
        return self._add("int", int(defval), title, min=minval, max=maxval, step=step)

    def float(self, defval, title=None, minval=None, maxval=None, step=0.1):
        return self._add("float", float(defval), title, min=minval, max=maxval, step=step)

    def bool(self, defval, title=None):
        return self._add("bool", bool(defval), title)

    def string(self, defval, title=None, options=None):
        return self._add("string", str(defval), title, options=list(options or []))

    def source(self, defval="close", title="Source"):
        return self._add("source", str(defval), title)


input = _Input()


def plot(series, title=None, color="#2962FF", linewidth=1, style="line"):
    title = title or f"Plot {len(_state.plots) + 1}"
    _state.plots.append(dict(title=title, values=series, color=color, width=int(linewidth), style=style))


def hline(price, title=None, color="#787B86", linestyle="dashed"):
    price = _num(price)
    if price is not None:
        title = title or f"Level {len(_state.hlines) + 1}"
        _state.hlines.append(dict(title=title, value=price, color=color, style=linestyle))


class Bars:
    """The chart's bars, oldest first: ``bars.close``, ``bars["hl2"]``…"""

    def __init__(self, d):
        self.time = d["time"]
        for name in ("open", "high", "low", "close", "volume"):
            setattr(self, name, [nan if v is None else v for v in d[name]])
        self.hl2 = [(h + l) / 2 for h, l in zip(self.high, self.low)]
        self.hlc3 = [(h + l + c) / 3 for h, l, c in zip(self.high, self.low, self.close)]
        self.ohlc4 = [(o + h + l + c) / 4 for o, h, l, c in zip(self.open, self.high, self.low, self.close)]
        self.hlcc4 = [(h + l + c + c) / 4 for h, l, c in zip(self.high, self.low, self.close)]

    def __len__(self):
        return len(self.close)

    def __getitem__(self, name):
        return getattr(self, name)

    def df(self):
        """The bars as a pandas DataFrame (needs pandas)."""
        import pandas as pd

        return pd.DataFrame(
            {"open": self.open, "high": self.high, "low": self.low, "close": self.close, "volume": self.volume},
            index=pd.to_datetime(self.time, unit="ms", utc=True),
        )


class _Strategy:
    """``strategy(...)`` declares a strategy; ``strategy.entry``,
    ``strategy.exit`` and ``strategy.close`` place orders on the bars where
    their ``when`` series is true (orders fill on the next bar's open, as
    on TradingView)."""

    long = "long"
    short = "short"

    def __call__(
        self,
        title,
        shorttitle=None,
        overlay=True,
        initial_capital=100000.0,
        default_qty_type="fixed",
        default_qty_value=1.0,
        pyramiding=0,
        commission_type="percent",
        commission_value=0.0,
        slippage=0,
        process_orders_on_close=False,
        currency="",
    ):
        _state.meta_.update(name=title, short=shorttitle or title, overlay=bool(overlay), kind="strategy")
        _state.meta_["props"] = dict(
            initial_capital=float(initial_capital),
            default_qty_type=default_qty_type,
            default_qty_value=float(default_qty_value),
            pyramiding=int(pyramiding),
            commission_type=commission_type,
            commission_value=float(commission_value),
            slippage=int(slippage),
            process_orders_on_close=bool(process_orders_on_close),
            currency=currency,
        )

    def _n(self):
        return _state.n

    def _each(self, x):
        """A number, or a series of them, as one value per bar."""
        if hasattr(x, "tolist"):
            x = x.tolist()
        if isinstance(x, (list, tuple)):
            return [_num(v) for v in _series(x, self._n())]
        return [_num(x)] * self._n()

    def _place(self, kind, when, common, **series):
        series = {k: self._each(v) for k, v in series.items()}
        for i, on in enumerate(_bools(when, self._n())):
            if on:
                _state.intents.append(dict(common, bar=i, type=kind, **{k: v[i] for k, v in series.items()}))

    def _long(self, direction):
        if direction in ("long", True):
            return True
        if direction in ("short", False):
            return False
        raise ValueError(f"The direction must be strategy.long or strategy.short, not {direction!r}.")

    def entry(self, id, direction, when=True, qty=None, limit=None, stop=None, comment=None):
        common = dict(id=str(id), long=self._long(direction), comment=comment)
        self._place("entry", when, common, qty=qty, limit=limit, stop=stop)

    def order(self, id, direction, when=True, qty=None, limit=None, stop=None, comment=None):
        common = dict(id=str(id), long=self._long(direction), comment=comment)
        self._place("order", when, common, qty=qty, limit=limit, stop=stop)

    def exit(self, id, from_entry="", when=True, qty_percent=100.0, profit=None, loss=None, limit=None, stop=None,
             comment=None):
        common = dict(id=str(id), from_entry=str(from_entry), comment=comment)
        self._place("exit", when, common, qty_percent=qty_percent, profit=profit, loss=loss, limit=limit, stop=stop)

    def close(self, id, when=True, comment=None):
        self._place("close", when, dict(id=str(id), comment=comment))

    def close_all(self, when=True, comment=None):
        self._place("close_all", when, dict(comment=comment))


strategy = _Strategy()

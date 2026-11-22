#!/usr/bin/env python3
"""Writes crates/core/src/venue_days.rs: trading days for the stock
exchanges whose indices Opentern lists (calendar::Venue), from the open
`exchange_calendars` project (Apache-2.0,
https://github.com/gerrymanoim/exchange_calendars).

For each exchange: its regular hours and lunch break (local time), its
holidays (weekdays without a session) with their names where the project
has them, the days whose hours differ (half days, late opens, special
sessions), and how far ahead the list is known. Holidays the project
doesn't name show as "Market holiday". Exchanges whose holidays
follow fixed rules are written to the end of RULES_TO; those that announce
them each year (lunar calendars) as far as the project knows.

Run it each year (and whenever an exchange announces next year's holidays),
check the diff, and ship it with a release:

    uv run --with exchange_calendars scripts/exchange-holidays.py

US (NYSE rules) and Indian (NSE's list) holidays are kept by hand in
calendar.rs. Additions exchange_calendars doesn't have yet go in
EXTRA_HOLIDAYS / EXTRA_SPECIAL below.
"""

import datetime as dt
import json
import pathlib
import re
import sys

import exchange_calendars as xcals
import pandas as pd

ROOT = pathlib.Path(__file__).resolve().parent.parent
OUT = ROOT / "crates" / "core" / "src" / "venue_days.rs"
START = "2025-01-01"
# Exchanges with rule-based holidays: written this far ahead.
RULES_TO = "2031-12-31"

# Venue name in calendar.rs → (exchange_calendars code, rule-based?)
VENUES = {
    "Tokyo": ("XTKS", True),
    "London": ("XLON", True),
    "Frankfurt": ("XETR", True),
    "Paris": ("XPAR", True),
    "Zurich": ("XSWX", True),
    "HongKong": ("XHKG", False),
    "Shanghai": ("XSHG", False),
    "Seoul": ("XKRX", False),
    "Taipei": ("XTAI", False),
    "Singapore": ("XSES", False),
    "Sydney": ("XASX", True),
    "Toronto": ("XTSE", True),
    "SaoPaulo": ("BVMF", True),
}


# Announcements exchange_calendars doesn't have yet. Add each one here as the
# exchange publishes it (check the exchange's own notice), then rerun.
EXTRA_HOLIDAYS = {
    # KRX: Constitution Day is a holiday again from 2026 (KRX notice, May 2026).
    "Seoul": [(dt.date(2026, 7, 17), "Constitution Day")],
}
# (date, open, close, lunch) — SGX's half days on the eves of Lunar New Year,
# Christmas and New Year: trading until 12:00.
EXTRA_SPECIAL = {
    "Singapore": [
        (dt.date(y, m, d), "09:00", "12:00", None)
        for (y, m, d) in [(2025, 1, 28), (2025, 12, 24), (2025, 12, 31), (2026, 2, 16), (2026, 12, 24), (2026, 12, 31)]
    ],
}


# The project's names → plain English ones.
RENAME = {
    "Ano Novo": "New Year's Day",
    "Dia da Confraternizacao Universal": "New Year's Day",
    "Carnaval Segunda": "Carnival",
    "Carnaval Terca": "Carnival",
    "Dia dos Finados": "All Souls' Day",
    "Dia Nacional de Zumbi e da Consciencia Negra": "Black Consciousness Day",
    "Dia Trabalho": "Labour Day",
    "Independencia": "Independence Day",
    "Natal": "Christmas Day",
    "Christmas": "Christmas Day",
    "Nossa Senhora de Aparecida": "Our Lady of Aparecida",
    "Proclamacao da Republica": "Republic Proclamation Day",
    "Vespera Natal": "Christmas Eve",
    "Kings's Birthday": "King's Birthday",
    "Weekend Boxing Day": "Boxing Day",
    "Weekend Christmas": "Christmas Day",
    "Seollal (New Year's Day by the lunar)": "Lunar New Year",
    "Chuseok (Korean Thanksgiving Day)": "Chuseok",
    "Canadian Thanksgiving": "Thanksgiving",
    "Health and Sports Day": "Sports Day",
    "Labor Day": "Labour Day",
}


def clean(name):
    """A holiday's name without the project's notes ("(2000 onwards)",
    "(+1 day)", "(Dec 31)")."""
    name = re.sub(r"\s*\((\d{4} onwards|[+-]\d+ days?|(Jan|Dec) \d+)\)", "", name).strip()
    return RENAME.get(name, name)


def calendar(code, rules):
    if rules:
        try:
            return xcals.get_calendar(code, start=START, end=RULES_TO)
        except Exception as e:  # noqa: BLE001 - fall back to the default span
            print(f"  {code}: {e}; using its default span", file=sys.stderr)
    return xcals.get_calendar(code, start=START)


def local(ts, tz):
    return ts.tz_convert(tz).strftime("%H:%M") if not pd.isna(ts) else None


def mode(values):
    return pd.Series(values).mode().iloc[0]


def names(cal, start, end):
    """Holiday names the project knows (rule-based ones)."""
    out = {}
    try:
        hs = cal.regular_holidays.holidays(start, end, return_name=True)
        for d, n in hs.items():
            out[d.date()] = str(n)
    except Exception:  # noqa: BLE001 - not every calendar names them
        pass
    return out


def hm(s):
    h, m = s.split(":")
    return f"({int(h)}, {int(m)})"


def venue_rows(venue, code, rules):
    cal = calendar(code, rules)
    tz = cal.tz
    sched = cal.schedule
    opens = [local(t, tz) for t in sched["open"]]
    closes = [local(t, tz) for t in sched["close"]]
    breaks = [
        (local(a, tz), local(b, tz)) if not pd.isna(a) else None
        for a, b in zip(sched["break_start"], sched["break_end"])
    ]
    # Regular hours: those of the last year (hours change now and then:
    # Tokyo closes at 15:30 since Nov 2024).
    recent = sched.index >= sched.index[-1] - pd.Timedelta(days=365)
    r_open = mode([o for o, k in zip(opens, recent) if k])
    r_close = mode([c for c, k in zip(closes, recent) if k])
    r_break = mode([str(b) for b, k in zip(breaks, recent) if k])
    r_break = None if r_break == "None" else eval(r_break)  # noqa: S307 - our own repr
    sessions = set(d.date() for d in sched.index)
    first, last = sched.index[0].date(), sched.index[-1].date()
    named = names(cal, pd.Timestamp(first), pd.Timestamp(last))
    holidays = []
    d = first
    while d <= last:
        if d.weekday() < 5 and d not in sessions:
            holidays.append((d, clean(named.get(d, ""))))
        d += dt.timedelta(days=1)
    for d, n in EXTRA_HOLIDAYS.get(venue, []):
        if first <= d <= last and d not in [h for h, _ in holidays]:
            holidays.append((d, n))
    holidays.sort()
    special = []
    for day, o, c, b in zip(sched.index, opens, closes, breaks):
        regular_day = day.weekday() < 5 and o == r_open and c == r_close and (b == r_break or (b is None and r_break is None))
        if not regular_day:
            special.append((day.date(), o, c, b))
    extra = {d for d, *_ in EXTRA_SPECIAL.get(venue, [])}
    special = [x for x in special if x[0] not in extra] + EXTRA_SPECIAL.get(venue, [])
    special.sort(key=lambda x: x[0])
    off = {d for d, _ in holidays}
    special = [x for x in special if x[0] not in off]
    return {
        "venue": venue,
        "code": code,
        "tz": str(tz),
        "open": r_open,
        "close": r_close,
        "break": r_break,
        "until": last,
        "holidays": holidays,
        "special": special,
    }


def rust(rows):
    out = [
        "// Generated by scripts/exchange-holidays.py from exchange_calendars",
        "// (Apache-2.0, https://github.com/gerrymanoim/exchange_calendars).",
        "// Don't edit by hand: run the script again (see its header).",
        "",
        "use super::calendar::VenueDays;",
        "",
    ]
    for r in rows:
        brk = f"Some(({hm(r['break'][0])}, {hm(r['break'][1])}))" if r["break"] else "None"
        out.append(f"/// {r['code']}, {r['tz']}.")
        out.append(f"pub const {r['venue'].upper()}: VenueDays = VenueDays {{")
        out.append(f"    tz: \"{r['tz']}\",")
        out.append(f"    open: {hm(r['open'])},")
        out.append(f"    close: {hm(r['close'])},")
        out.append(f"    lunch: {brk},")
        u = r["until"]
        out.append(f"    until: ({u.year}, {u.month}, {u.day}),")
        out.append("    holidays: &[")
        for d, n in r["holidays"]:
            out.append(f"        ({d.year}, {d.month}, {d.day}, {json.dumps(n, ensure_ascii=False)}),")
        out.append("    ],")
        out.append("    special: &[")
        for d, o, c, b in r["special"]:
            bb = f"Some(({hm(b[0])}, {hm(b[1])}))" if b else "None"
            out.append(f"        ({d.year}, {d.month}, {d.day}, {hm(o)}, {hm(c)}, {bb}),")
        out.append("    ],")
        out.append("};")
        out.append("")
    return "\n".join(out)


def main():
    print(f"exchange_calendars {xcals.__version__}", file=sys.stderr)
    rows = []
    for venue, (code, rules) in VENUES.items():
        r = venue_rows(venue, code, rules)
        print(
            f"  {venue:10} {code} {r['open']}-{r['close']} lunch {r['break']} until {r['until']}: "
            f"{len(r['holidays'])} holidays, {len(r['special'])} special days",
            file=sys.stderr,
        )
        rows.append(r)
    OUT.write_text(rust(rows))
    print(f"wrote {OUT.relative_to(ROOT)}", file=sys.stderr)


if __name__ == "__main__":
    main()

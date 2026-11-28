# Ideas for later

Ideas the maintainer wants kept but not built yet. Don't start one without
their go; when one is built, move its notes into the right `docs/notes/` file and delete it here.

## Scripts recalculate in the background (Nov 2026)
Today a script's full recalculation (adding it, new inputs, a new symbol or
interval, more history) runs on the UI thread in `Chart::run_study`, so a
heavy script freezes the chart for that long (a 20,000-bar strategy can take
0.3 s or more). Built-ins are fast enough to stay as they are.

- **How:** for a script study (`inst.factory()` is some, not external), a full
  run builds a fresh `Instance` (`with_inputs`), sends it with a copy of the
  bars, sessions, interval and its links to `rayon::spawn`, and the chart keeps
  drawing the old outputs. The result comes back on a channel the chart owns,
  polled each frame (`App::frame`, every chart via `each_chart`); the worker
  calls `request_repaint`. On arrival: drop it if a newer run was asked for or
  the bars were replaced since (an epoch bumped by `recompute_all`), otherwise
  `update` it to catch up with bars that came meanwhile, swap it in and
  `touch()`.
- **Live ticks** keep updating the old instance while a run is out.
- **Order:** a study that reads another (`study_order`, `refresh_readers`)
  waits until the one it reads has come back, then runs with the new links.
- **Crashes:** wrap the run in `catch_unwind` (rayon aborts on a panic) and
  report "The script crashed." as the screener does.
- **Tests:** run synchronously while the chart has no egui context (unit
  tests), so the existing chart tests don't change; check `drive.rs` tests and
  stills, which may expect a script's lines on the first frames.

## Strategies: bar magnifier and every-tick recalculation (Nov 2026)
Waiting on outside pull requests for issues #5 (`calc_on_order_fills`) and #6
(Strategy Report Overview chart); the maintainer reviews them first. These two
follow on from #5:

- **Bar magnifier (`use_bar_magnifier`)**: fill stops, limits and exits along
  the real price path from a lower timeframe's bars inside each bar (as
  TradingView picks it: 1h → 1m, 1D → 5m and so on), instead of the guessed
  open → nearer extreme → other → close. The biggest accuracy gain for
  backtests, and it makes `calc_on_order_fills` trustworthy (its extra runs
  happen at real points rather than guessed ones). Needs lower-timeframe
  history for the whole range (the feeds `request.security_lower_tf` uses),
  so it's slower and limited by how far back intraday data goes.
- **Every-tick recalculation (`calc_on_every_tick`)**: on the live bar, rerun
  the strategy on each price update rather than once at the close, so it can
  act before the bar closes (as on TradingView live; history still runs at
  closes). The engine already recomputes the forming bar on a clone
  (`Instance::update`); the broker would need the same commit/rollback per
  tick that #5 needs per fill (ordinary variables roll back, `varip` keeps).
- Today both settings are only noted in the script's log ("Opentern doesn't
  simulate …").

## Send to a nearby device, AirDrop style (Nov 2026)
Instead of typing a six-digit code, Send shows a radar (rings pulsing out
from the centre, a dot with a name per device) of the Opentern apps nearby
that are on Import → From another device. Click one to send.

- **Discovery flips:** the receiver answers probes with its device name only
  while its import screen is open; the sender searches continuously and
  drops devices that go quiet. Reuses `core/src/handoff.rs` (broadcast,
  `PORT`, firewall help, encrypted transfer). Keep the typed-address fallback:
  broadcast fails on guest Wi-Fi, client isolation and some mesh routers.
- **Security: a click alone isn't enough.** With no code, nothing proves who
  is on the other end. A LAN device could announce itself as "MacBook" and get
  the workspace (watchlists, scripts, alerts with webhook URLs and the Telegram
  chat id, paper accounts; never keychain secrets), or sit in the middle.
  Planned fix, as Bluetooth pairing and AirDrop do it:
  - The receiver gets "<device> wants to send settings" with Accept / Decline.
  - Both screens show the same short code derived from the key exchange, to glance
    at and compare, not type (a mismatch means someone is in the middle).
  - Optional second step: remember a device after one confirmed send (keep
    each other's keys), so later sends are one click + Accept.
- Effort: about a day or two. The device-to-device transfer itself is still
  unverified across two real machines (Windows Firewall, macOS local network
  prompt); discovery leans on that even more.

## Upstox Analytics Token for complete Indian history (Nov 2026)
Yahoo's bars skip some special sessions (Muhurat on 8 Nov 2026, 12 Nov
2023, 2010–2016's, the 2024 drill Saturdays; see docs/notes/data.md "Special
sessions"). NSE's own files would fill them, but NSE's terms forbid the app
fetching them, so the licensed route is the user's own broker.

- Upstox's normal access tokens expire daily, but its newer **Analytics
  Token** is long-lived and read-only, meant for historical data without
  the daily login (announced on the developer portal; length and cost not
  checked). Read-only also means it can't place orders.
- Opentern's Upstox adapter (`crates/data/src/upstox.rs`) expects a normal
  token and has never run against a real account. Supporting the
  Analytics Token is a small change: accept it in Settings → Data
  providers and use it for history (and quotes, if it allows them).
- **Check first** with a real account: do Upstox's daily candles include
  the Muhurat sessions (e.g. SBIN on 8 Nov 2026: 1023.20 / 1025.00 /
  1015.10 / 1016.00)? If not, this doesn't help.
- Others considered (from memory, unchecked): Angel One and Fyers are free
  but need a daily or fortnightly login (or storing a TOTP secret); Dhan's
  data API may need a paid plan; Zerodha and Groww are paid with daily
  tokens.

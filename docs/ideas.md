# Ideas for later

Ideas the maintainer wants kept but not built yet. Don't start one without
their go; when one is built, move its notes into `CLAUDE.md` and delete it here.

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
2023, 2010–2016's, the 2024 drill Saturdays; see CLAUDE.md "Special
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

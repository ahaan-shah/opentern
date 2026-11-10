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

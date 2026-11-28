# Notes: alerts, background alerts, earnings, EXT

Moved out of `CLAUDE.md` (Nov 2026) to keep the briefing short. Keep it current: update it in the same commit as the code.

## Phase 8 (done): alerts
- `opentern_engine::alerts`: `Alert` (price / drawn line / study plot / script),
  `Op` (crossing, up, down, greater, less), `Freq` (only once, once per bar, once per bar
  close, every time ≤ 1/s), `Watch::feed` (pure, tested). DB v4: `alerts`, `alert_log`
  (latest 2,000).
- `app/src/alerts.rs`: evaluated every frame but only fed new (value, level, bar)
  triples; price alerts work for any symbol with a price (chart or quotes), plot alerts
  need the study on the chart; line alerts follow their drawing; script alerts
  (`alert()`, `alertcondition`) fire only for alerts newer than when they were armed
  (deduped by bar time and message). Fired → log, a notification (its card pops up; `notes.rs`), and a
  delivery thread: desktop notification + sound (`notify-rust`), webhook (JSON or text
  body; logs only the host), Telegram (bot token in the OS keychain via `keyring`,
  service `opentern`; chat id in settings `telegram_chat_id`). Nothing is sent when
  `OPENTERN_HEADLESS` is set.
- UI: Create/Edit alert dialog (Alt+A, top-bar Alert, the widget's +), Alerts widget
  (state, pause/restart, edit, delete on hover), Alerts log tab (clear log).
- **Gotcha (hover flicker):** a row that shows buttons on hover must test
  `ui.rect_contains_pointer(rect)`, not `resp.hovered()`: the button takes the
  hover, the row hides it, and it flickers every frame.
- **Gotcha:** egui's `Event::Text` carries no modifiers; type-to-search must check
  `i.modifiers` or Alt+A/Alt+T also type into symbol search. Tests must send
  `Event::ModifiersChanged` to model held keys.
- Email (Oct 2026): `lettre` over SMTP with rustls (STARTTLS, or TLS on 465);
  password in the keychain (`email-smtp-password`), server/port/login/recipient
  in settings (`email_smtp_*`, `email_to`); "Send a test email" in the dialog.
  Never tested against a real mail server.
- `opentern --daemon` and off-chart alerts: done Nov 2026 (see "Background
  alerts" above). Not yet: sound files.

## Background alerts, earnings, EXT (0.2.5, Nov 2026)
- **`opentern --daemon`** (`daemon.rs`): watches alerts with no window.
  One evaluator per workspace: the app holds `app.lock` (std file locks,
  hence `rust-version` 1.89), the daemon `alerts.lock`; the daemon checks
  `app.lock` every ~200 ms and exits when the app opens (the app waits up
  to 1.5 s for it before loading alerts); it also exits when no alert is
  active. **One window per workspace** (`instance.rs`, Nov 2026): `main`
  takes `app.lock` before opening the window; a second launch finds it held,
  reads the port and token the first wrote to `<workspace>/instance`, asks it
  over loopback to come forward and quits. A restart (update, staged import)
  sets `OPENTERN_RELAUNCH` so the new process waits up to 15 s for the lock
  instead. The app spawns it on exit (own process group / detached) when
  `alerts_background` isn't "off" and an alert is active; `alerts_at_login`
  writes an XDG autostart file / LaunchAgent plist / HKCU Run value
  (macOS and Windows untested). `alerts_fired_away` counts fires for the
  app's "N alerts fired while Opentern was closed" note.
- **`watcher.rs`**: UI-free alert evaluation: a hub session per (symbol,
  tf) and the alert's study rebuilt from `Alert::study_spec` (built-in id
  or `pine:file`, inputs) via `pine_host::instance_for_file` (Pine with the
  folder's libraries, Luau/JS/TS; Python needs the app; no feeds, so
  `request.security` on other symbols is `na` there). The app uses it for
  alerts whose symbol or study isn't on the active chart. Alerts now save
  `sym` and `study_spec` (`App::fill_alert_context`, refreshed on exit).
  `Alerts::{evaluate, fire, save}` are shared by both.
- **Earnings** (`data/src/earnings.rs`): Yahoo's visualization API
  (history, but stale after mid-2025), Nasdaq's earnings-surprise (last 4
  quarters, US), Yahoo `calendarEvents` (next date), merged within 5
  days; carried in `Corporate::earnings`, drawn as E markers (upcoming
  ones placed past the last bar by time). Demo stocks get made-up ones.
- **EXT**: `Request::Open { ext }`; US stocks on Yahoo get
  `includePrePost=true`, cached as `<id>#ext`; `Chart::ext` (saved),
  shading in `draw_ext_hours`. Still `ext`.
- **Journal: shelved** (the maintainer, 2 Nov 2026: "don't want it"). The
  module is in `git stash` ("shelved: trade journal"); its glue (tab, App
  field, `Snap::Journal`, notes' trade link) was removed. Don't bring it
  back unless asked.
- **Alert lines** (Nov 2026, off by default: `ChartStyle::alert_lines`, with
  `alert_color`/`alert_width`/`alert_style` shared by all): `App::alert_lines` puts the
  chart's active `Condition::Price` alerts on `Chart::alerts` (rebuilt only when a hash
  of what they show changes; every grid chart via `poll_other_charts`). `chart/ui.rs`
  draws them under the scale tags with their price tag and, near the pointer, a name
  pill at the left; with the cross a line drags (`Action::MoveAlert` →
  `App::move_alert`) and right-clicks (`Action::AlertMenu` → `alert_line_menu`: edit,
  delete, colour, style, width). Their price tags do the same (`ChartUi::alert_tags`,
  the tags' rects as last painted, made clickable after the scale's own response so
  they win the press). The menu closes on a press only over a layer below
  `Order::Foreground` (`layer_id_at`), so the colour picker and tooltips never close it.

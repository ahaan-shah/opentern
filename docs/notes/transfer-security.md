# Notes: export and import, security, fuzzing

Moved out of `CLAUDE.md` (Nov 2026) to keep the briefing short. Keep it current: update it in the same commit as the code.

## Export and import (Nov 2026, 0.2.6)
- **Settings → Export and import** (`app/src/transfer.rs`, `Cat::Transfer`). Export view:
  a checkbox per `transfer::Item` (all on, with counts), Export (a save dialog) and Send to
  another device. Import view: From a file (or drop one on the page) / From another device.
  Both end in the same review: items in the file, Merge or Replace, Import and restart.
- **The file** (`core/src/transfer.rs`, `.opentern`): a zip of `manifest.json`,
  `data/<item>.json` and `scripts/<path>`. JSON, never the raw database, so versions can
  differ; bump `FORMAT` only for what older apps can't read (they ask for an update).
  Never included: secrets (keychain), folder settings, login items, agent accounts and
  chats, bar cache. A **new setting that belongs to an item must be added to that item's
  `gather`/`apply_item`**, or it won't travel. Script paths are checked on the way in.
- **Merge** matches by name (watchlists, layouts, templates, paper accounts), symbol
  (drawings), ticker (indices), path (scripts) or what the alert watches (`alert_identity`);
  the file wins and the rest stays. **Replace** clears the ticked items first.
- **Importing happens at startup:** the app verifies the file, saves everything to
  `<workspace>/backups/before-import-*.opentern` (latest 5), stages it
  (`import/pending.*`) and restarts (`Updater::relaunch`, which drops `OPENTERN_STILL`).
  `main` applies it before the store opens (`daemon::apply_pending_import` takes
  `app.lock` first); the database commits as one transaction; the outcome is reported
  once by `App::import_outcome` (settings key `import_result`). The daemon isn't started
  while an import is pending. A failed import isn't retried.
- **Security: a file is untrusted input, whoever sent it** (a signature couldn't help:
  there's no identity to check). Don't weaken these. `app::open_url` opens only `http(s)` links (`may_open`). `manifest.json` is only for display;
  warnings use `transfer::inspect` (the real contents). Python has no sandbox, so an import
  records each `.py`'s SHA-256 in `PYTHON_HELD_KEY`; `App::start_script` (the only thing that
  spawns Python) won't start a held one, and only the editor's Run, pressed twice, clears it
  (`trust_python`; the agent's `add_script` can't). By content, so no path spelling gets
  past it. Any script path from saved data goes through `transfer::stays_inside`. Alert
  delivery settings only fill blanks (the keychain's mail password goes to whatever server
  they name). Symbols name files in logos and snapshots: sanitise. Luau: `require`,
  `loadstring` and friends are removed in `luau.rs` (mlua adds them); `script/tests/sandbox.rs`
  tries to break out. Handoff: `is_local` on every peer, `MAX_TRIES` per search,
  `HANDSHAKE_LIMIT`. Attacked for real before 0.2.6 (hostile files in real windows, loopback
  fuzzing). Known limits: 5 guesses at a 6-digit code per offer; a LAN device can end an offer
  or a search (fails safe); the agents can
  read imported scripts, so a hostile script could try to talk them into something.
- **Network input is untrusted too** (reviewed Nov 2026). Don't weaken: TLS
  verification is only skipped for the IBKR gateway on this machine
  (`ibkr::is_loopback` compares the host exactly: `contains("://localhost")`
  passed `localhost.attacker.example`); requests carrying keys never follow
  redirects (`get_json_with`; Alpaca's keys are custom headers ureq would
  forward); websocket messages ≤ 8 MB (`WS_MAX_MESSAGE`); downloaded logos:
  SVG `<image>` only from data in the file (`logos::svg_options`; usvg's
  default reads any path from disk), PNGs sized from the header before
  decoding (≤ 2048 px, 32 MB); CSV text cells that start like a formula get
  `'` (`quant::export::cell`; order names come from scripts); the agents'
  system prompt says tool results are data, not instructions. Links open
  only as http(s) (`may_open`); file names from symbols are sanitised; the
  only listener is the transfer handoff while it's sharing.
- **Fuzzing** (Nov 2026; stable Rust, no cargo-fuzz: `opentern_core::fuzzing`
  mutates JSON structure, text and bytes, catches panics, says where):
  every provider reply parser (`data/src/fuzz.rs`), Pine scripts (all of
  `tests/corpus`, compiled and run: `pine/tests/fuzz.rs`), logos (SVG kinds,
  PNG, logoapi), `.opentern` imports (bytes, and good zips with mutated
  JSON and hostile script paths). Short runs in `cargo test`; long:
  `OPENTERN_FUZZ_ROUNDS=40000 cargo test --all fuzz` (Pine:
  `OPENTERN_FUZZ_TRACE=<file>` keeps the last input, for aborts). Found and
  fixed: seconds → ms overflow on absurd times (`provider::secs_ms`/
  `sane_ms`), a `ta.*` length reserving petabytes (`Hist::new`),
  `str.repeat` of repeats, stack overflow on deep nesting in the Pine
  parser (`MAX_DEPTH`; each link of an operator chain counts as a level)
  and in formulas (`MAX_NESTING`, `MAX_TOKENS`: a 1,000-member custom index
  fits; `Expr::eval`/`render` loop down the left-leaning chain). Stack
  tests run on 256 KB: Windows' main thread is 1 MB and its debug frames
  are the largest (CI caught a 2,000-member formula overflowing there). Aborts (out of memory, stack) can't be
  caught: bound every allocation and recursion a script or reply controls.
- **cargo-deny** (`deny.toml`, CI job `deny`): crates.io only, an allow-list
  of licences, yanked crates fail. Known and accepted: rustix's xattr and
  tokio's `AsyncFd` "unsound" notices (no fix yet; neither is called), and
  zbus/zvariant unmaintained (via notify-rust).
- **Permissions:** Windows Firewall and macOS Local Network ask once. `NetHelp` offers the way
  back: `handoff::firewall` runs `opentern --allow-firewall` elevated (clears the exe's block
  rules, adds rule "Opentern transfer" for port 47811; stays after uninstalling on a per-user
  install, harmless) and macOS opens its settings page. Windows CI runs the real `netsh` and
  elevation (`firewall` tests, `CI` set). Stills: `send@windows`, `notfound@mac`.
- **Send** (`core/src/handoff.rs`): sender listens only while its code shows (TCP and UDP
  `PORT` 47811, TCP falls back to any). Receiver broadcasts, or takes a typed address once
  the search fails. SPAKE2 on the six-digit code, ChaCha20-Poly1305 per direction; five
  wrong codes end the offer, the code lasts 15 minutes. The sender's code keeps working
  after a dropped connection. **Only run between processes on one machine so far; real
  two-machine runs, Windows Firewall and macOS's local network prompt are unverified.**
- Stills: `settings:transfer` plus `transfer:import|saved|send:<code>|receive:<code>|
  review:<path>|import-now:<path>|sending:<pct>|sent|send-failed|receiving:<pct>|notfound|wrong`.
  Two windows with `send:` and `receive:` of one code run a real transfer.
- **Future idea (not started):** AirDrop-style "nearby devices" radar instead of the
  code; design and security notes in `docs/ideas.md`.

# Notes: shipping, releases, website

Moved out of `CLAUDE.md` (Nov 2026) to keep the briefing short. Keep it current: update it in the same commit as the code.

## Phase 14: repo, CI, packages, screenshots

**Phase 14 (started 16 Oct 2026, the maintainer's go):** `ahaan-shah/opentern`
   on GitHub, **public since 21 Oct 2026** (v0.1.2, the maintainer's go after a
   full history scan for secrets and personal data: clean). CI runs leaks, `cargo audit`, fmt, clippy
   and tests on Linux, plus clippy and tests on macOS 15 and Windows
   (since 0.2.5). Pushes that only change Markdown or `docs/` (except
   `docs/scripting.md`, compiled in) skip `ci.yml`; the leak check is its own
   `leaks.yml` and runs on every push. `release.yml` on a `v*` tag builds Linux x86-64 +
   ARM64 tarballs (`scripts/package-linux.sh`), a universal macOS `.app` as
   `.dmg` + `.zip` (`scripts/bundle-macos.sh`, ad-hoc signed, not notarized),
   Windows x64 + ARM64 zips and an Inno Setup installer
   (`scripts/package-windows.ps1`, `packaging/windows/opentern.iss`; never
   change its AppId), plus `SHA256SUMS`, `install.sh` (Linux) and `install.ps1`
   (Windows). Asset names are a contract with the updater.
   `THIRD-PARTY-LICENSES.txt` (`scripts/third-party-licenses.py`; re-run after
   dependency or asset changes) ships in every package and in the binary
   (`opentern --licenses`).
   The updater falls back to the `gh` CLI on a 404 (private repo) for the check
   and the download, and now **requires** SHA256SUMS. Settings → About: logo,
   version, Check for updates, the launch-check toggle. Screenshots (README
   and website) are `site/media/*.webp`: demo data, taken with `OPENTERN_STILL`
   plus `OPENTERN_STILL_NOINPUT=1` (ignores mouse and keys, so no stray
   crosshair or typed keys), a fresh `--data-dir` per shot, `theme:<name>` for
   the theme deck; 2160x1350 grabs scaled to 1880x1175 WebP. Release history is
   in `CHANGELOG.md`. The macOS and Windows builds have been tested on a real
   Mac and PC and work. Website shots
   of a tiled window: a temporary Hyprland rule floats new opentern windows
   at 1080x675 (`hyprctl eval 'otShot = hl.window_rule({ name =
   "opentern-shot", match = { class = "^opentern$" }, float = true, size =
   {1080, 675}, move = {17, 59} })'`, then `otShot:set_enabled(false)`); grab
   only after `hyprctl activewindow` shows your own pid; `widget:none` closes
   the side panel. Still to do: notarizing the macOS app.

## Updates and the install lines (0.3.6)
- **The updater never waits without a limit** (`update::Limits`): 10 s to
  connect (ureq shares it across a host's addresses, so one that doesn't
  answer falls through to the next), 30 s for a response, and downloads in
  2 MB HTTP ranges with 90 s each, a stalled or dropped piece asked for again
  from the byte it got to (four tries without progress). Before 0.3.6 there
  were no limits: one of GitHub's four download addresses not answering on the
  maintainer's network held each request for Linux's ~2 min TCP timeout, and
  the bar sat at 100% while `SHA256SUMS` was fetched. Tests: `update::tests`
  (a local server that stalls, drops, ignores ranges, never answers) and
  `live_release_download` (ignored; the real release through GitHub's redirect).
- Steps (`update::Step`) show in Settings → About and on the top-bar button; a
  failed install is `Phase::Retry` with the reason in words
  (`updater::reason`) and Try again. Work folders of interrupted updates
  (`.opentern-update-<pid>` beside the binary) are cleared on launch.
- **`install.sh`**: curl with `--connect-timeout 10 --retry 4 --speed-limit
  1024 --speed-time 30`; a download or unpack that can't be saved in
  `$TMPDIR` (a full `/tmp`, or a per-user quota) moves to the cache folder;
  curl's exit codes are said in words. **`install.ps1`**: 60 s per request,
  four tries, `%LOCALAPPDATA%` when the temp folder is full, `SHA256SUMS`
  required. CI's `installers` job runs both for real on every push (Linux with
  a 1 MB temp folder).
- `bundle-macos.sh` sizes the `.dmg` itself: `hdiutil`'s own guess fell short
  once the app grew (0.3.5's first tag).

## Website
**Screenshots (refreshed 24 Nov 2026):** demo data in a seed workspace
(`scripts/ma_cross.pine`, an MA Cross 12/30 strategy; five NSE sector indices
written into settings `custom_indices`), a fresh copy per shot, the window
floated to 1080x675 on ws10 and grabbed with `grim -T` (see
`CLAUDE.local.md`), then `magick -resize 1880x1175 -quality 86` to WebP.
Stills: home `script:ma_cross.pine,add:bb,add:rsi` (+`light`, or
`theme:<name>` for the theme deck), `tab:script`, `tab:strategy r`,
`lab:heatmap`, `grid:Four`, `screen`, `tab:indices,max,widget:none`,
`symbol:NSE:RELIANCE,tf:1D,tab:seasonality,max,widget:none`,
`settings:general,scroll:196` (past Folders, whose paths would show),
`overview`, and the agent with `agent:claude,claude-model:sonnet,claude:<msg>`
plus `OPENTERN_CLAUDE` pointed at a throwaway stand-in CLI that replays one
scripted turn (like `claude/fake_claude.py`; keep it out of the repo).
`site/` (static: `index.html`, `style.css`, `site.js`, fonts, `media/`) in
Spotsie's style and Opentern's theme colours, published by `pages.yml` to
https://ahaan-shah.github.io/opentern/ on pushes touching `site/`. Install lines
must match `install.sh` / `install.ps1` and the README.

## Releasing (only when asked; same as Magpie and Spotsie)
1. Bump `version` in `Cargo.toml` (workspace) and every `opentern-* = { version
   = … }` path dep in `crates/*/Cargo.toml`, README's `OPENTERN_VERSION=`
   example; `cargo update -w`.
2. **What's new notes**: an entry for the version at the top of
   `whatsnew::RELEASES` (a test fails without it). Then the
   `CHANGELOG.md` entry (`## X.Y.Z (date)`; if there's a `## Unreleased`
   section, rename it and check it covers everything since the last tag): a short plain summary, then New /
   Changed / Fixed / Removed, each item leading with a bold user-facing result.
   The release notes are cut from it (the publish job fails without one). Never
   placeholder notes.
3. **Exchange holidays (yearly, and when an exchange announces next
   year's):** `uv run --with exchange_calendars scripts/exchange-holidays.py`,
   add one-off announcements to its `EXTRA_*` lists, check the diff of
   `venue_days.rs`; add NSE's new year to `NSE_HOLIDAYS` by hand.
   If dependencies or bundled assets changed: `python3
   scripts/third-party-licenses.py`. Run `cargo audit` (CI fails on advisories).
4. fmt / clippy / test / `scripts/check-leaks.sh`, plus the cross-checks
   against second sources (`cargo test -p opentern-data --test crosscheck --
   --ignored --nocapture --test-threads=1`: US closes = Nasdaq's to the cent,
   Binance vs Coinbase, NSE vs BSE listings, old prices pinned). Then commit, push `main`, and
   **wait until every CI job is green on that commit** before tagging
   (CI covers Linux, macOS and Windows since 0.2.5). Also look at the
   annotations of the last CI and Release runs (`gh run view <id>`) for
   runner-image deprecation notices: 0.2.5's first tag hit a scheduled
   macOS 14 brownout (GitHub retired it 2 Nov 2026) and its macOS job
   was cancelled before it started. Images are pinned; move `ci.yml` and
   `release.yml` together when one is announced for retirement.
5. `git tag -a vX.Y.Z -m "Opentern X.Y.Z" && git push origin vX.Y.Z`; watch
   `release.yml` with `gh run watch`. A `workflow_dispatch` run builds every
   platform without publishing (do one after changing packaging).
6. Don't install the new version locally (`install.sh` or otherwise) unless
   the maintainer asks.
**Gate before step 5 (tagging): the maintainer tests it** (their rule, 24 Nov 2026):
   build (`cargo build --profile fast`), open it for them on workspace 10
   (see `CLAUDE.local.md`), and wait for their go. Skip only when they
   say so, and even then suggest they try it first.
7. Asset names are a contract with `install.sh`, `install.ps1` and
   `update::asset_name`. Don't rename them.

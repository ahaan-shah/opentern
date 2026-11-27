#!/bin/sh
# Opentern installer for Linux and macOS (adapted from Magpie's).
#
#   curl -fsSL https://raw.githubusercontent.com/ahaan-shah/opentern/main/install.sh | sh
#
# To uninstall (your workspace is kept): add `-s -- --uninstall` after `sh`.
# Windows: install.ps1, or Opentern-windows-setup.exe from the releases.
#
# Environment:
#   OPENTERN_VERSION      install a specific version (e.g. 0.1.4) instead of the latest
#   OPENTERN_INSTALL_DIR  where the binary goes on Linux (default: ~/.local/bin)
#   OPENTERN_BASE_URL     download from a mirror instead of GitHub releases
set -eu

REPO="ahaan-shah/opentern"
VERSION="${OPENTERN_VERSION:-latest}"

# A terminal gets the logo and a drawn progress bar; a pipe or a log gets plain lines.
FANCY=""
if [ -t 1 ] && [ "${TERM:-dumb}" != dumb ]; then FANCY=1; fi
if [ -n "$FANCY" ] && [ -z "${NO_COLOR:-}" ]; then
    BOLD="$(printf '\033[1m')" DIM="$(printf '\033[2m')" RED="$(printf '\033[31m')" RESET="$(printf '\033[0m')"
    case "${COLORTERM:-}" in
    truecolor | 24bit) GREEN="$(printf '\033[38;2;34;181;108m')" TRUECOLOR=1 ;;
    *) case "${TERM:-}" in *256color*) GREEN="$(printf '\033[38;5;35m')" ;; *) GREEN="$(printf '\033[32m')" ;; esac ;;
    esac
else
    BOLD="" DIM="" RED="" RESET="" GREEN=""
fi
TRUECOLOR="${TRUECOLOR:-}"

if [ -n "$FANCY" ]; then
    say() { printf '  %s\n' "$*"; }
    ok() { printf '  %s✓%s %s\n' "$GREEN" "$RESET" "$*"; }
    die() { printf '\r\033[K\033[?25h  %serror%s %s\n' "$BOLD$RED" "$RESET" "$*" >&2; exit 1; }
else
    say() { printf 'opentern: %s\n' "$*"; }
    ok() { say "$*"; }
    die() { printf 'opentern: error: %s\n' "$*" >&2; exit 1; }
fi
need() { command -v "$1" >/dev/null 2>&1 || die "this installer needs '$1'"; }

# The app icon's two candles, a shade lighter at the top as on the icon.
banner() {
    [ -n "$FANCY" ] || return 0
    printf '\n'
    i=0
    while IFS= read -r row; do
        colour="$GREEN"
        if [ -n "$TRUECOLOR" ]; then
            colour="$(printf '\033[38;2;%d;%d;%dm' $((52 - i * 45 / 7)) $((211 - i * 80 / 7)) $((130 - i * 56 / 7)))"
        fi
        case $i in
        3) text="      ${BOLD}Opentern${RESET}" ;;
        4) text="      ${DIM}installer${RESET}" ;;
        *) text="" ;;
        esac
        printf '  %s%s%s%s\n' "$colour" "$row" "$RESET" "$text"
        i=$((i + 1))
    done <<'LOGO'
          ██  
  ██    ██████
██████  ██████
██  ██  ██████
██  ██  ██████
██████  ██████
  ██    ██████
          ██  
LOGO
    printf '\n'
}

# "~/.local/bin" reads better than the full path.
tilde() {
    case "$1" in
    "$HOME"/*) printf '~%s' "${1#"$HOME"}" ;;
    *) printf '%s' "$1" ;;
    esac
}

OS="$(uname -s)"
BIN_DIR="${OPENTERN_INSTALL_DIR:-$HOME/.local/bin}"
DATA="${XDG_DATA_HOME:-$HOME/.local/share}"
ICONS="$DATA/icons/hicolor"

refresh() {
    command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database -q "$DATA/applications" || true
    command -v gtk-update-icon-cache >/dev/null 2>&1 && gtk-update-icon-cache -q -t "$ICONS" 2>/dev/null || true
}

# Removes exactly what an install put there. The workspace stays, so
# reinstalling picks up where you left off.
uninstall() {
    case "$OS" in
    Linux)
        [ -e "$BIN_DIR/opentern" ] || [ -e "$DATA/applications/opentern.desktop" ] ||
            die "Opentern isn't installed in $BIN_DIR (set OPENTERN_INSTALL_DIR if you chose another folder)"
        rm -f "$BIN_DIR/opentern" "$DATA/applications/opentern.desktop" "$ICONS"/*/apps/opentern.svg
        # Left only if an in-app update was interrupted.
        rm -rf "$BIN_DIR"/.opentern-update-* "$BIN_DIR/.opentern.old"
        refresh
        say "Uninstalled $BIN_DIR/opentern"
        say "Your workspace is still in $DATA/opentern (delete it to remove that too)"
        ;;
    Darwin)
        found=""
        for app in /Applications/Opentern.app "$HOME/Applications/Opentern.app"; do
            [ -e "$app" ] || continue
            found=1
            rm -rf "$app" 2>/dev/null || die "couldn't remove $app (try: sudo rm -rf '$app')"
            say "Uninstalled $app"
        done
        [ -n "$found" ] || die "Opentern.app isn't in /Applications or ~/Applications"
        say "Your workspace is still in ~/Library/Application Support (delete its opentern folder to remove that too)"
        ;;
    *) die "unsupported OS: $OS (this installer is for Linux and macOS; on Windows use install.ps1)" ;;
    esac
}

case "${1:-}" in
"") ;;
--uninstall)
    uninstall
    exit 0
    ;;
*) die "unknown option: $1 (the only option is --uninstall)" ;;
esac

need uname
need curl
need awk

# Name the version up front (and download exactly that one) by following
# GitHub's redirect from /releases/latest to its tag.
if [ "$VERSION" = "latest" ] && [ -z "${OPENTERN_BASE_URL:-}" ]; then
    tag="$(curl -fsSLI --connect-timeout 10 --retry 2 -o /dev/null -w '%{url_effective}' "https://github.com/$REPO/releases/latest" 2>/dev/null || true)"
    case "$tag" in */tag/v*) VERSION="${tag##*/tag/v}" ;; esac
fi
if [ "$VERSION" = "latest" ]; then
    BASE="https://github.com/$REPO/releases/latest/download"
    SHOWN="Opentern"
else
    VERSION="${VERSION#v}"
    BASE="https://github.com/$REPO/releases/download/v$VERSION"
    SHOWN="Opentern $VERSION"
fi
BASE="${OPENTERN_BASE_URL:-$BASE}"

# A server that never answers, or a transfer that stops, is given up on and
# tried again instead of waiting for the system's two-minute TCP timeout.
CURL_OPTS="--connect-timeout 10 --retry 4 --retry-delay 1 --speed-limit 1024 --speed-time 30"

# Downloads go to a fresh folder in the temporary directory. If that has no
# room (a full /tmp, or a per-user quota on it), the user's cache folder.
TMP="$(mktemp -d 2>/dev/null || true)"
FALLBACK=""
PID=""
cleanup() {
    [ -n "$PID" ] && kill "$PID" 2>/dev/null
    [ -n "$FANCY" ] && printf '\033[?25h'
    [ -n "$TMP" ] && rm -rf "$TMP"
}
# Moves the downloads to the cache folder; false if that's where they are.
use_fallback() {
    [ -z "$FALLBACK" ] || return 1
    case "$OS" in
    Darwin) root="$HOME/Library/Caches" ;;
    *) root="${XDG_CACHE_HOME:-$HOME/.cache}" ;;
    esac
    mkdir -p "$root" 2>/dev/null || return 1
    new="$(mktemp -d "$root/opentern-install.XXXXXX" 2>/dev/null)" || return 1
    if [ -n "$TMP" ]; then
        # What's already downloaded comes along.
        for f in "$TMP"/*; do [ -f "$f" ] && mv "$f" "$new/" 2>/dev/null; done
        rm -rf "$TMP"
    fi
    TMP="$new"
    FALLBACK=1
}
[ -n "$TMP" ] || use_fallback || die "couldn't make a folder to download into"

# What a curl exit code means, in words.
why() {
    case "$1" in
    6) printf "couldn't look up the server (check your connection)" ;;
    7) printf "couldn't connect to the server (check your connection)" ;;
    22) printf "the server refused it" ;;
    23) printf "there's no room to save it in %s" "$(tilde "$(dirname "$TMP")")" ;;
    28) printf "the connection timed out" ;;
    35 | 60) printf "a secure connection couldn't be made" ;;
    56) printf "the connection dropped" ;;
    *) printf "curl error %s" "$1" ;;
    esac
}
trap cleanup EXIT
trap 'cleanup; trap - EXIT; exit 130' INT TERM

# One line of the bar: ■ for what's in, · for what's left, then % and MB.
draw() {
    got=0
    if [ -f "$TMP/$1" ]; then got="$(wc -c <"$TMP/$1" | tr -d ' ')"; fi
    # Redirects come first in the headers; the last length is the file's.
    total="$(tr -d '\r' <"$TMP/headers" 2>/dev/null | awk 'tolower($1) == "content-length:" { n = $2 } END { print n + 0 }')"
    cols="$(tput cols 2>/dev/null || echo 80)"
    width=32
    [ "$cols" -lt 70 ] && width=$((cols - 38))
    [ "$width" -lt 10 ] && width=10
    if [ "$total" -gt 0 ]; then
        [ "$got" -gt "$total" ] && got="$total"
        filled=$((got * width / total))
        pct="$((got * 100 / total))%"
        size="$(mb "$got") / $(mb "$total") MB"
    else
        filled=0 pct="" size="$(mb "$got") MB"
    fi
    done_part="" left_part="" n=0
    while [ $n -lt $width ]; do
        if [ $n -lt $filled ]; then done_part="${done_part}■"; else left_part="${left_part}·"; fi
        n=$((n + 1))
    done
    printf '\r\033[K  %s%s%s%s%s  %4s  %s%s%s' "$GREEN" "$done_part" "$RESET$DIM" "$left_part" "$RESET" "$pct" "$DIM" "$size" "$RESET"
}

mb() {
    tenths=$(($1 * 10 / 1048576))
    printf '%d.%d' $((tenths / 10)) $((tenths % 10))
}

# Downloads $1 into $TMP; `bar` draws the progress bar in a terminal. A
# download that can't be saved is tried once more in the cache folder.
fetch() {
    while :; do
        if fetch_once "$@"; then return 0; else status=$?; fi
        if [ "$status" -eq 23 ] && use_fallback; then
            say "No room in the temporary folder: downloading to $(tilde "$(dirname "$TMP")") instead"
            continue
        fi
        die "couldn't download $1: $(why "$status")"
    done
}

fetch_once() {
    if [ -z "$FANCY" ] || [ "${2:-}" != bar ]; then
        [ -n "$FANCY" ] || say "Downloading $1"
        # shellcheck disable=SC2086
        curl -fsSL $CURL_OPTS "$BASE/$1" -o "$TMP/$1" 2>/dev/null
        return
    fi
    : >"$TMP/headers" 2>/dev/null || return 23
    printf '\033[?25l'
    # shellcheck disable=SC2086
    curl -fsSL $CURL_OPTS -D "$TMP/headers" "$BASE/$1" -o "$TMP/$1" 2>/dev/null &
    PID=$!
    while kill -0 "$PID" 2>/dev/null; do
        draw "$1"
        sleep 0.1 2>/dev/null || sleep 1
    done
    if wait "$PID"; then status=0; else status=$?; fi
    PID=""
    if [ "$status" -ne 0 ]; then
        printf '\r\033[K\033[?25h'
        return "$status"
    fi
    draw "$1"
    printf '\033[?25h\n\n'
}

# Runs `$1`, a function that unpacks into $TMP; if that runs out of room, it
# runs again in the cache folder (the download comes along).
unpack() {
    "$1" 2>/dev/null && return 0
    if use_fallback; then
        say "No room in the temporary folder: unpacking in $(tilde "$(dirname "$TMP")") instead"
        "$1" 2>/dev/null && return 0
    fi
    die "couldn't unpack the download (is there room in $(tilde "$(dirname "$TMP")")?)"
}
untar() { tar -xzf "$TMP/$NAME.tar.gz" -C "$TMP"; }
unzip_app() { unzip -q "$TMP/$ASSET" -d "$TMP"; }

start() {
    banner
    if [ -n "$FANCY" ]; then
        printf '  %sInstalling%s %s%s%s\n\n' "$DIM" "$RESET" "$BOLD" "$SHOWN" "$RESET"
    else
        say "Installing $SHOWN"
    fi
}

sha256() {
    if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1"; else shasum -a 256 "$1"; fi | cut -d' ' -f1
}

verify() {
    fetch SHA256SUMS
    expected="$(grep " $1\$" "$TMP/SHA256SUMS" | cut -d' ' -f1)"
    [ -n "$expected" ] || die "$1 isn't listed in the release's SHA256SUMS"
    [ "$expected" = "$(sha256 "$TMP/$1")" ] || die "checksum mismatch for $1"
    ok "Checksum verified"
}

case "$OS" in
Linux)
    need tar
    ARCH="$(uname -m)"
    case "$ARCH" in
    x86_64 | amd64) ARCH=x86_64 ;;
    aarch64 | arm64) ARCH=aarch64 ;;
    *) die "there's no build for $ARCH (you can build from source: https://github.com/$REPO#building-from-source)" ;;
    esac

    NAME="opentern-$ARCH-unknown-linux-gnu"
    start
    fetch "$NAME.tar.gz" bar
    verify "$NAME.tar.gz"
    unpack untar
    SRC="$TMP/$NAME"

    mkdir -p "$BIN_DIR" "$DATA/applications" "$ICONS/scalable/apps"
    # Replace, never write into, a running copy's file.
    install -m 755 "$SRC/opentern" "$BIN_DIR/.opentern.new"
    mv -f "$BIN_DIR/.opentern.new" "$BIN_DIR/opentern"
    install -m 644 "$SRC/opentern.svg" "$ICONS/scalable/apps/opentern.svg"
    for s in 16 24; do
        [ -f "$SRC/opentern-$s.svg" ] || continue
        mkdir -p "$ICONS/${s}x${s}/apps"
        install -m 644 "$SRC/opentern-$s.svg" "$ICONS/${s}x${s}/apps/opentern.svg"
    done
    sed "s|^Exec=opentern|Exec=$BIN_DIR/opentern|" "$SRC/opentern.desktop" >"$DATA/applications/opentern.desktop"
    chmod 644 "$DATA/applications/opentern.desktop"
    refresh

    ok "Installed to $(tilde "$BIN_DIR")"
    [ -n "$FANCY" ] && printf '\n'
    case ":$PATH:" in
    *":$BIN_DIR:"*) say "Run it with ${BOLD}opentern${RESET}, or open it from your app menu" ;;
    *) say "$(tilde "$BIN_DIR") isn't on your PATH: add it, or open Opentern from your app menu" ;;
    esac
    [ -n "$FANCY" ] && printf '\n'
    true
    ;;
Darwin)
    need unzip
    ASSET="Opentern-macos-universal.zip"
    start
    fetch "$ASSET" bar
    verify "$ASSET"
    unpack unzip_app
    if [ -w /Applications ]; then DEST=/Applications; else DEST="$HOME/Applications"; mkdir -p "$DEST"; fi
    rm -rf "$DEST/Opentern.app"
    mv "$TMP/Opentern.app" "$DEST/"
    # Downloaded with curl, so there's no quarantine flag, but clear it just in case.
    xattr -dr com.apple.quarantine "$DEST/Opentern.app" 2>/dev/null || true
    ok "Installed to $(tilde "$DEST")"
    [ -n "$FANCY" ] && printf '\n'
    say "Open it from Launchpad, or run ${BOLD}open -a Opentern${RESET}"
    [ -n "$FANCY" ] && printf '\n'
    true
    ;;
*)
    die "unsupported OS: $OS (this installer is for Linux and macOS; on Windows use install.ps1)"
    ;;
esac

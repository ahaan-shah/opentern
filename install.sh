#!/bin/sh
# Opentern installer for Linux and macOS (adapted from Magpie's).
#
#   curl -fsSL https://raw.githubusercontent.com/ahaan-shah/opentern/main/install.sh | sh
#
# To uninstall (your workspace is kept): add `-s -- --uninstall` after `sh`.
# Windows: install.ps1, or Opentern-windows-setup.exe from the releases.
#
# Environment:
#   OPENTERN_VERSION      install a specific version (e.g. 0.1.3) instead of the latest
#   OPENTERN_INSTALL_DIR  where the binary goes on Linux (default: ~/.local/bin)
#   OPENTERN_BASE_URL     download from a mirror instead of GitHub releases
set -eu

REPO="ahaan-shah/opentern"
VERSION="${OPENTERN_VERSION:-latest}"

say() { printf '\033[1;36mopentern\033[0m %s\n' "$*"; }
die() { printf '\033[1;31merror\033[0m %s\n' "$*" >&2; exit 1; }
need() { command -v "$1" >/dev/null 2>&1 || die "this installer needs '$1'"; }

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
        say "uninstalled $BIN_DIR/opentern"
        say "your workspace is still in $DATA/opentern (delete it to remove that too)"
        ;;
    Darwin)
        found=""
        for app in /Applications/Opentern.app "$HOME/Applications/Opentern.app"; do
            [ -e "$app" ] || continue
            found=1
            rm -rf "$app" 2>/dev/null || die "couldn't remove $app (try: sudo rm -rf '$app')"
            say "uninstalled $app"
        done
        [ -n "$found" ] || die "Opentern.app isn't in /Applications or ~/Applications"
        say "your workspace is still in ~/Library/Application Support (delete its opentern folder to remove that too)"
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

if [ "$VERSION" = "latest" ]; then
    BASE="https://github.com/$REPO/releases/latest/download"
else
    BASE="https://github.com/$REPO/releases/download/v${VERSION#v}"
fi
BASE="${OPENTERN_BASE_URL:-$BASE}"

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT INT TERM

fetch() {
    say "downloading $1"
    curl -fL --progress-bar "$BASE/$1" -o "$TMP/$1" || die "couldn't download $BASE/$1"
}

sha256() {
    if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1"; else shasum -a 256 "$1"; fi | cut -d' ' -f1
}

verify() {
    fetch SHA256SUMS
    expected="$(grep " $1\$" "$TMP/SHA256SUMS" | cut -d' ' -f1)"
    [ -n "$expected" ] || die "$1 isn't listed in the release's SHA256SUMS"
    [ "$expected" = "$(sha256 "$TMP/$1")" ] || die "checksum mismatch for $1"
    say "checksum ok"
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
    fetch "$NAME.tar.gz"
    verify "$NAME.tar.gz"
    tar -xzf "$TMP/$NAME.tar.gz" -C "$TMP"
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

    say "installed $("$BIN_DIR/opentern" --version 2>/dev/null || echo opentern) to $BIN_DIR"
    case ":$PATH:" in
    *":$BIN_DIR:"*) ;;
    *) say "note: $BIN_DIR isn't on your PATH. Add it, or open Opentern from your app menu" ;;
    esac
    say "run it with: opentern"
    ;;
Darwin)
    need unzip
    ASSET="Opentern-macos-universal.zip"
    fetch "$ASSET"
    verify "$ASSET"
    unzip -q "$TMP/$ASSET" -d "$TMP"
    if [ -w /Applications ]; then DEST=/Applications; else DEST="$HOME/Applications"; mkdir -p "$DEST"; fi
    rm -rf "$DEST/Opentern.app"
    mv "$TMP/Opentern.app" "$DEST/"
    # Downloaded with curl, so there's no quarantine flag, but clear it just in case.
    xattr -dr com.apple.quarantine "$DEST/Opentern.app" 2>/dev/null || true
    say "installed $DEST/Opentern.app"
    say "open it from Launchpad, or run: open -a Opentern"
    ;;
*)
    die "unsupported OS: $OS (this installer is for Linux and macOS; on Windows use install.ps1)"
    ;;
esac

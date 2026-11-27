#!/usr/bin/env bash
# Builds this checkout and installs it as "Opentern Dev" (Linux), beside any
# released Opentern: ~/.local/bin/opentern-dev plus an app launcher entry.
# Each run replaces the last one, as an update replaces the app. Running
# copies keep going; the new build starts next time it's opened.
#
# A binary named opentern-dev is a development copy (opentern_core::dev_copy):
# it keeps its own workspace (~/.local/share/opentern-dev), never the real
# one, and never updates itself.
#
#   scripts/dev-install.sh            build with the fast profile and install
#   scripts/dev-install.sh --remove   take it away again (its workspace stays)
set -euo pipefail
cd "$(dirname "$0")/.."

BIN_DIR="${OPENTERN_INSTALL_DIR:-$HOME/.local/bin}"
DATA="${XDG_DATA_HOME:-$HOME/.local/share}"
APPS="$DATA/applications"
ICONS="$DATA/icons/hicolor"

refresh() {
    command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database -q "$APPS" || true
    command -v gtk-update-icon-cache >/dev/null 2>&1 && gtk-update-icon-cache -q -t "$ICONS" 2>/dev/null || true
}

if [ "${1:-}" = "--remove" ]; then
    rm -f "$BIN_DIR/opentern-dev" "$APPS/opentern-dev.desktop" "$ICONS"/*/apps/opentern-dev.svg
    refresh
    echo "Removed Opentern Dev"
    exit 0
fi

cargo build --profile fast -p opentern

mkdir -p "$BIN_DIR" "$APPS" "$ICONS/scalable/apps"
# Replace, never write into, a running copy's file.
install -m 755 target/fast/opentern "$BIN_DIR/.opentern-dev.new"
mv -f "$BIN_DIR/.opentern-dev.new" "$BIN_DIR/opentern-dev"

install -m 644 packaging/linux/opentern.svg "$ICONS/scalable/apps/opentern-dev.svg"
for s in 16 24; do
    mkdir -p "$ICONS/${s}x${s}/apps"
    install -m 644 "packaging/linux/opentern-$s.svg" "$ICONS/${s}x${s}/apps/opentern-dev.svg"
done

sed -e "s|^Name=Opentern$|Name=Opentern Dev|" \
    -e "s|^Exec=opentern$|Exec=$BIN_DIR/opentern-dev|" \
    -e "s|^Icon=opentern$|Icon=opentern-dev|" \
    -e "s|^StartupWMClass=opentern$|StartupWMClass=opentern-dev|" \
    -e "s|^Comment=.*|Comment=Development build of Opentern ($(git rev-parse --short HEAD)$(git diff --quiet HEAD || echo '+changes'))|" \
    packaging/linux/opentern.desktop >"$APPS/.opentern-dev.desktop.new"
mv -f "$APPS/.opentern-dev.desktop.new" "$APPS/opentern-dev.desktop"
chmod 644 "$APPS/opentern-dev.desktop"
refresh

echo "Installed Opentern Dev to $BIN_DIR/opentern-dev"

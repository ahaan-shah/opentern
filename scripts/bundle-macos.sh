#!/usr/bin/env bash
# Builds Opentern.app (universal: Apple Silicon + Intel) plus a .dmg and .zip
# (adapted from Magpie's). The zip's name is a contract with the in-app
# updater (crates/core/src/update.rs). Run on macOS:
#   ./scripts/bundle-macos.sh [version]
set -euo pipefail
cd "$(dirname "$0")/.."
VERSION="${1:-$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)}"
OUT=dist
APP="$OUT/Opentern.app"

rustup target add aarch64-apple-darwin x86_64-apple-darwin >/dev/null
for t in aarch64-apple-darwin x86_64-apple-darwin; do
    MACOSX_DEPLOYMENT_TARGET=11.0 cargo build --release --locked --target "$t" -p opentern
done

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
lipo -create -output "$APP/Contents/MacOS/opentern" \
    target/aarch64-apple-darwin/release/opentern target/x86_64-apple-darwin/release/opentern
sed "s/__VERSION__/$VERSION/g" packaging/macos/Info.plist > "$APP/Contents/Info.plist"

# Icon: 1024px PNG -> .iconset -> .icns
ICONSET="$OUT/opentern.iconset"
rm -rf "$ICONSET" && mkdir -p "$ICONSET"
for s in 16 32 128 256 512; do
    sips -z $s $s packaging/linux/icon-1024.png --out "$ICONSET/icon_${s}x${s}.png" >/dev/null
    sips -z $((s * 2)) $((s * 2)) packaging/linux/icon-1024.png --out "$ICONSET/icon_${s}x${s}@2x.png" >/dev/null
done
iconutil -c icns "$ICONSET" -o "$APP/Contents/Resources/opentern.icns"
cp THIRD-PARTY-LICENSES.txt LICENSE "$APP/Contents/Resources/"
rm -rf "$ICONSET"

# Ad-hoc signature so Apple Silicon will run it.
codesign --force --deep --sign - "$APP"

(cd "$OUT" && rm -f Opentern-macos-universal.zip && ditto -c -k --keepParent Opentern.app Opentern-macos-universal.zip)

STAGE="$OUT/dmg"
rm -rf "$STAGE" && mkdir -p "$STAGE"
cp -R "$APP" "$STAGE/"
ln -s /Applications "$STAGE/Applications"
rm -f "$OUT/Opentern-macos-universal.dmg"
hdiutil create -volname "Opentern" -srcfolder "$STAGE" -ov -format UDZO "$OUT/Opentern-macos-universal.dmg" >/dev/null
rm -rf "$STAGE"
echo "Built $OUT/Opentern-macos-universal.dmg and .zip (v$VERSION)"

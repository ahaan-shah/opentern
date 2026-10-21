#!/usr/bin/env bash
# Builds a release tarball for one Linux target (adapted from Magpie's):
#   ./scripts/package-linux.sh x86_64-unknown-linux-gnu
set -euo pipefail
cd "$(dirname "$0")/.."
TARGET="${1:-$(rustc -vV | sed -n 's/host: //p')}"
cargo build --release --locked --target "$TARGET" -p opentern
NAME="opentern-$TARGET"
STAGE="dist/$NAME"
rm -rf "$STAGE" && mkdir -p "$STAGE"
cp "target/$TARGET/release/opentern" "$STAGE/"
cp packaging/linux/opentern.desktop "$STAGE/"
cp packaging/linux/opentern.svg packaging/linux/opentern-16.svg packaging/linux/opentern-24.svg "$STAGE/"
cp LICENSE README.md THIRD-PARTY-LICENSES.txt "$STAGE/"
tar -C dist -czf "dist/$NAME.tar.gz" "$NAME"
rm -rf "$STAGE"
echo "Built dist/$NAME.tar.gz"

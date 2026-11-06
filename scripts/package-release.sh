#!/bin/sh
# Build a release and lay it out the way `whet` and install.sh expect to download it:
#
#   <out>/latest                                   version of the newest release
#   <out>/install.sh, install.ps1                  the bootstrap scripts
#   <out>/keen/<ver>/keen-<ver>-<target>.tar.gz    toolchain (+ .sha256)
#   <out>/whet/<ver>/whet-<target>                 the manager binary (+ .sha256)
#
# Usage: scripts/package-release.sh [--out DIR] [--target TRIPLE] [--no-latest] [--skip-build]
# Upload <out> to the server named by WHET_DEFAULT_DIST_SERVER (set it when building `whet`).
set -eu

OUT=dist
TARGET=""
LATEST=1
BUILD=1
while [ $# -gt 0 ]; do
    case "$1" in
        --out) OUT=$2; shift 2 ;;
        --target) TARGET=$2; shift 2 ;;
        --no-latest) LATEST=0; shift ;;
        --skip-build) BUILD=0; shift ;;
        -h | --help) sed -n '2,12p' "$0"; exit 0 ;;
        *) echo "unknown option: $1" >&2; exit 2 ;;
    esac
done

cd "$(dirname "$0")/.."
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1)
HOST=$(rustc -vV | sed -n 's/^host: //p')
if [ -z "$TARGET" ]; then TARGET=$HOST; fi

if [ "$BUILD" = 1 ]; then
    if [ "$TARGET" = "$HOST" ]; then
        cargo build --release
    else
        cargo build --release --target "$TARGET"
    fi
fi
if [ "$TARGET" = "$HOST" ]; then BIN=target/release; else BIN=target/$TARGET/release; fi

sha256() {
    if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1"; else shasum -a 256 "$1"; fi
}
write_sum() { # FILE  ->  FILE.sha256 containing "<hex>  <basename>"
    (cd "$(dirname "$1")" && sha256 "$(basename "$1")") >"$1.sha256"
}

NAME="keen-$VERSION-$TARGET"
mkdir -p "$OUT/keen/$VERSION" "$OUT/whet/$VERSION"
STAGE=$(mktemp -d)
trap 'rm -rf "$STAGE"' EXIT
mkdir -p "$STAGE/$NAME/bin" "$STAGE/$NAME/share"
cp "$BIN/keen" "$STAGE/$NAME/bin/keen"
cp README.md "$STAGE/$NAME/share/README.md"
cp docs/*.md "$STAGE/$NAME/share/" 2>/dev/null || true
cp -R examples "$STAGE/$NAME/share/examples"
tar -czf "$OUT/keen/$VERSION/$NAME.tar.gz" -C "$STAGE" "$NAME"
write_sum "$OUT/keen/$VERSION/$NAME.tar.gz"

cp "$BIN/whet" "$OUT/whet/$VERSION/whet-$TARGET"
write_sum "$OUT/whet/$VERSION/whet-$TARGET"

cp scripts/install.sh scripts/install.ps1 "$OUT/"
if [ "$LATEST" = 1 ]; then printf '%s\n' "$VERSION" >"$OUT/latest"; fi
echo "packaged keen $VERSION for $TARGET in $OUT/"

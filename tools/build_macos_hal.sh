#!/bin/sh
set -eu
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$ROOT"
if [ "$(uname -s)" != Darwin ]; then
  echo 'NeonMix HAL builds on macOS only' >&2
  exit 1
fi
if [ "${NEONMIX_HAL_DIAGNOSTICS:-0}" = 1 ]; then
  tools/dev cargo build --release --locked -p neonmix-hal --features diagnostics
else
  tools/dev cargo build --release --locked -p neonmix-hal
fi
BUNDLE="$ROOT/artifacts/macos/NeonMixHAL.driver"
mkdir -p "$BUNDLE/Contents/MacOS"
mkdir -p "$BUNDLE/Contents/Resources"
cp drivers/macos/Info.plist "$BUNDLE/Contents/Info.plist"
cp target/release/libneonmix_hal.dylib "$BUNDLE/Contents/MacOS/NeonMixHAL"
cp vendor/tympan-aspl/LICENSE-MIT "$BUNDLE/Contents/Resources/TYMPAN-LICENSE-MIT"
cp vendor/tympan-aspl/LICENSE-APACHE "$BUNDLE/Contents/Resources/TYMPAN-LICENSE-APACHE"
codesign --force --sign - "$BUNDLE"
codesign --verify --strict "$BUNDLE"
lipo -verify_arch arm64 "$BUNDLE/Contents/MacOS/NeonMixHAL"
plutil -lint "$BUNDLE/Contents/Info.plist"
nm -gU "$BUNDLE/Contents/MacOS/NeonMixHAL" | rg '_NeonMixHalFactory$'
file "$BUNDLE/Contents/MacOS/NeonMixHAL"
printf '%s\n' "$BUNDLE"

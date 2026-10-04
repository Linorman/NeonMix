#!/bin/sh
# Capture settled frames of every desktop page from recorded public state.
# Preview mode starts no background and plays no audio.
# usage: tools/ui_preview_shots.sh OUTDIR [WIDTH HEIGHT] [FIXTURE]
# Live motion is pinned by NEONMIX_SCREENSHOT_PHASE (default 1.25 s) so
# repeated captures of the signal graph are identical.
# Requires: tools/dev cargo build -p neonmix-desktop --release --features screenshot
set -eu
cd "$(dirname "$0")/.."
OUT=$1; W=${2:-1100}; H=${3:-760}
F=${4:-docs/evidence/macos-e00-e07-20261001-132347/mixer/ui-real-state.json}
mkdir -p "$OUT"
export NEONMIX_SCREENSHOT_PHASE="${NEONMIX_SCREENSHOT_PHASE:-1.25}"
for p in live mixer hub sender devices diagnostics about; do
  rm -f "$OUT/$p-$W.png"
  NEONMIX_SCREENSHOT_TO="$OUT/$p-$W.png" tools/dev target/release/neonmix-desktop \
    --preview-page "$p" --preview-data "$F" --width "$W" --height "$H" >/dev/null 2>&1 &
  PID=$!
  # The first launch after a build can take several seconds (font load).
  i=0; while [ ! -f "$OUT/$p-$W.png" ] && [ $i -lt 120 ]; do sleep 0.25; i=$((i+1)); done
  sleep 0.3; kill $PID 2>/dev/null || true; wait $PID 2>/dev/null || true
  [ -f "$OUT/$p-$W.png" ] || { echo "no capture for $p" >&2; exit 1; }
done

#!/bin/sh
# CI-only Ubuntu sysroot: download/extract packages into the checkout, never apt install.
set -eu
NEONMIX_ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
NEONMIX_NATIVE="$NEONMIX_ROOT/.local/native"
mkdir -p "$NEONMIX_NATIVE/debs" "$NEONMIX_NATIVE/root" "$NEONMIX_ROOT/artifacts"
cd "$NEONMIX_NATIVE/debs"
apt-get download libpipewire-0.3-dev libpipewire-0.3-0t64 libspa-0.2-dev libasound2-dev libasound2t64 libxkbcommon-dev libwayland-dev libx11-dev libxrandr-dev libxi-dev libgl-dev libglx-dev fonts-noto-cjk
for package in ./*.deb; do
  dpkg-deb -x "$package" "$NEONMIX_NATIVE/root"
  dpkg-deb -f "$package" Package Version >> "$NEONMIX_ROOT/artifacts/native-packages.txt"
done
# Relocate only the downloaded .pc files; preinstalled system dependencies remain readable.
python3 - "$NEONMIX_NATIVE/root" <<'PY'
from pathlib import Path
import sys
root=Path(sys.argv[1])
for file in root.rglob('*.pc'):
    text=file.read_text().replace('=/usr',f'={root}/usr')
    file.write_text(text)
PY
if [ -n "${GITHUB_ENV:-}" ]; then
  echo "PKG_CONFIG_PATH=$NEONMIX_NATIVE/root/usr/lib/x86_64-linux-gnu/pkgconfig:$NEONMIX_NATIVE/root/usr/share/pkgconfig" >> "$GITHUB_ENV"
  echo "LIBRARY_PATH=$NEONMIX_NATIVE/root/usr/lib/x86_64-linux-gnu" >> "$GITHUB_ENV"
  echo "LD_LIBRARY_PATH=$NEONMIX_NATIVE/root/usr/lib/x86_64-linux-gnu" >> "$GITHUB_ENV"
fi

#!/bin/sh
set -eu
NEONMIX_ROOT=/home/parallels/NeonMix
export PATH="$NEONMIX_ROOT/.local/rust/bin:$NEONMIX_ROOT/.local/native/root/usr/bin:$PATH"
export PKG_CONFIG="$NEONMIX_ROOT/.local/native/root/usr/bin/pkgconf"
export LIBCLANG_PATH=/usr/lib/llvm-18/lib
export BINDGEN_EXTRA_CLANG_ARGS="-resource-dir=$NEONMIX_ROOT/.local/native/root/usr/lib/llvm-18/lib/clang/18"
export XDG_RUNTIME_DIR=/run/user/1000
export DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/1000/bus
export CARGO_BUILD_JOBS=2
cd "$NEONMIX_ROOT"
exec tools/dev "$@"

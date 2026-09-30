#!/bin/sh
# Explicit user-authorized activation. Refuse to interrupt another NeonMix audio probe.
set -eu
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
if [ "$(uname -s)" != Darwin ] || [ "$(id -u)" -ne 0 ]; then
  echo 'HAL activation requires macOS administrator authentication' >&2
  exit 1
fi
check_idle() {
  if /bin/ps -axo pid=,command= | /usr/bin/awk '/neonmix-(hub|audio) (serve|send|capture|play)/ {print $1; found=1} END {exit(found?0:1)}'; then
    echo 'NeonMix audio processes are still running; service restart was deferred' >&2
    exit 1
  fi
}
check_idle
/bin/sh "$ROOT/tools/install_macos_hal.sh" install
check_idle
/usr/bin/killall coreaudiod
printf 'Requested Core Audio restart; verify the NeonMix UID before testing\n'

#!/bin/sh
# Run the reviewed staged bundle as an explicit system installation. Never builds as root.
set -eu
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
BUNDLE="$ROOT/artifacts/macos/NeonMixHAL.driver"
DEST=/Library/Audio/Plug-Ins/HAL/NeonMixHAL.driver
MODE=${1:-install}
if [ "$(uname -s)" != Darwin ] || [ "$(id -u)" -ne 0 ]; then
  echo 'HAL installation requires macOS administrator authentication' >&2
  exit 1
fi
if [ -L "$DEST" ]; then
  echo 'Refusing to replace a symlink at the NeonMix HAL installation path' >&2
  exit 1
fi
if [ -e "$DEST" ]; then
  ID=$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$DEST/Contents/Info.plist")
  if [ "$ID" != com.neonmix.audio.driver ]; then
    echo 'Installed bundle identity does not belong to NeonMix' >&2
    exit 1
  fi
fi
case "$MODE" in
  install)
    ID=$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$BUNDLE/Contents/Info.plist")
    [ "$ID" = com.neonmix.audio.driver ]
    /usr/bin/codesign --verify --strict "$BUNDLE"
    /usr/bin/lipo -verify_arch arm64 "$BUNDLE/Contents/MacOS/NeonMixHAL"
    STAGE="${DEST}.staging-$$"
    trap '/bin/rm -rf "$STAGE"' EXIT HUP INT TERM
    /usr/bin/ditto "$BUNDLE" "$STAGE"
    /usr/sbin/chown -R root:wheel "$STAGE"
    /bin/chmod -R go-w "$STAGE"
    /usr/bin/codesign --verify --strict "$STAGE"
    if [ -e "$DEST" ]; then
      BACKUP="$ROOT/artifacts/macos/install-backups/$(date +%Y%m%d-%H%M%S)-$$.driver"
      /bin/mkdir -p "$(dirname "$BACKUP")"
      /usr/bin/ditto "$DEST" "$BACKUP"
      OWNER=$(/usr/bin/stat -f %u "$ROOT")
      /usr/sbin/chown -R "$OWNER" "$ROOT/artifacts/macos/install-backups"
      /bin/rm -rf "$DEST"
      printf 'Previous bundle saved in %s\n' "$BACKUP"
    fi
    /bin/mv "$STAGE" "$DEST"
    /usr/bin/codesign --verify --strict "$DEST"
    printf 'Installed %s; audio service has not been restarted\n' "$DEST"
    ;;
  uninstall)
    /bin/rm -rf "$DEST"
    printf 'Removed %s; audio service has not been restarted\n' "$DEST"
    ;;
  *) echo 'Usage: install_macos_hal.sh install|uninstall' >&2; exit 1 ;;
esac

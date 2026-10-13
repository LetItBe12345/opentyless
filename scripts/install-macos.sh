#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
APP_DIR="${OPENTYLESS_APPLICATIONS_DIR:-/Applications}"
APP="$APP_DIR/OpenTyless.app"
CONFIG_DIR="${XDG_CONFIG_HOME:-$HOME/.config}/opentyless"
BIN_DIR="$HOME/.local/bin"
[[ "$(uname -s)" == Darwin ]] || { echo 'macOS required' >&2; exit 1; }
if [[ -x "$APP/Contents/MacOS/opentyless-rs" ]]; then
  STATUS="$(cd "$CONFIG_DIR" && "$APP/Contents/MacOS/opentyless-rs" status 2>/dev/null || true)"
  if [[ "$STATUS" == *'daemon_running: yes'* ]] && { [[ "$STATUS" == *'"recording": true'* ]] || [[ "$STATUS" == *'"last_event": "transcribing"'* ]]; }; then
    echo 'Please finish recording/transcription before updating.' >&2
    exit 1
  fi
fi
"$ROOT/scripts/build-macos.sh"
mkdir -p "$CONFIG_DIR" "$BIN_DIR" "$APP_DIR"
if [[ ! -f "$CONFIG_DIR/.env" ]]; then
  cp "$ROOT/macos/env.example" "$CONFIG_DIR/.env"
fi
chmod 600 "$CONFIG_DIR/.env"
# Stop the previous version before replacing its files. Keep config/output.
launchctl bootout "gui/$(id -u)/local.opentyless.tray" 2>/dev/null || true
launchctl bootout "gui/$(id -u)/local.opentyless.daemon" 2>/dev/null || true
# Also stop a menu app opened manually from Finder.
pkill -x OpenTyless 2>/dev/null || true
if [[ -d "$APP" ]]; then
  mv "$APP" "$APP_DIR/OpenTyless.backup.$(date +%Y%m%d%H%M%S).app"
fi
ditto "$ROOT/target/macos/OpenTyless.app" "$APP"
codesign --verify --deep --strict "$APP"
for NAME in opentyless opentyless-rs; do
  if [[ -e "$BIN_DIR/$NAME" && ! -L "$BIN_DIR/$NAME" ]]; then
    mv "$BIN_DIR/$NAME" "$BIN_DIR/$NAME.backup.$(date +%Y%m%d%H%M%S)"
  fi
  ln -sfn "$APP/Contents/MacOS/opentyless-rs" "$BIN_DIR/$NAME"
done
# Avoid loading an unrelated .env from the source checkout.
cd "$CONFIG_DIR"
"$BIN_DIR/opentyless-rs" install-service --enable
"$BIN_DIR/opentyless-rs" install-tray-service --enable
"$BIN_DIR/opentyless-rs" doctor
printf '\nInstalled %s\nConfig: %s/.env\n' "$APP" "$CONFIG_DIR"

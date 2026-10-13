#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
[[ "$(uname -s)" == Darwin ]] || { echo 'macOS required' >&2; exit 1; }
cd "$ROOT"
cargo build --release --locked
APP="$ROOT/target/macos/OpenTyless.app"
mkdir -p "$APP/Contents/MacOS"
cp macos/Info.plist "$APP/Contents/Info.plist"
cp target/release/opentyless-rs "$APP/Contents/MacOS/opentyless-rs"
xcrun swiftc -swift-version 5 -O -target "$(uname -m)-apple-macosx13.0" macos/OpenTyless.swift -o "$APP/Contents/MacOS/OpenTyless" -framework AppKit -framework AVFoundation -framework Carbon
codesign --force --deep --sign - "$APP"
codesign --verify --deep --strict "$APP"
printf 'Built: %s\n' "$APP"

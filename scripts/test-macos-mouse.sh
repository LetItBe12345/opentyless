#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TEST_DIR="$(mktemp -d)"
trap 'rm -rf "$TEST_DIR"' EXIT
xcrun swiftc -swift-version 5 "$ROOT/macos/MouseShortcuts.swift" \
  "$ROOT/tests/macos_mouse_shortcuts.swift" -o "$TEST_DIR/mouse-shortcuts-tests" \
  -framework AppKit -framework Carbon
"$TEST_DIR/mouse-shortcuts-tests"

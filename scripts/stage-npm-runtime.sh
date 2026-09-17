#!/usr/bin/env bash
set -euo pipefail

PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
RUNTIME_SOURCE_DIR="${PROJECT_ROOT}/lib/runtime/source"
BINARY_PATH="${PROJECT_ROOT}/target/release/opentyless-rs"
HOTKEY_PATH="${PROJECT_ROOT}/scripts/hotkey-toggle.sh"

if [[ "${OPENTYLESS_SKIP_BUILD:-0}" != "1" ]]; then
  cargo build --release --manifest-path "${PROJECT_ROOT}/Cargo.toml"
fi

[[ -f "${BINARY_PATH}" ]] || {
  echo "[opentyless-npm] missing binary: ${BINARY_PATH}" >&2
  exit 1
}

rm -rf "${RUNTIME_SOURCE_DIR}"
mkdir -p "${RUNTIME_SOURCE_DIR}"
install -m 0755 "${BINARY_PATH}" "${RUNTIME_SOURCE_DIR}/opentyless-rs"
install -m 0755 "${HOTKEY_PATH}" "${RUNTIME_SOURCE_DIR}/opentyless-hotkey-toggle"
install -m 0644 "${PROJECT_ROOT}/README.md" "${RUNTIME_SOURCE_DIR}/README.md"
install -m 0644 "${PROJECT_ROOT}/README.zh-CN.md" "${RUNTIME_SOURCE_DIR}/README.zh-CN.md"
if [[ -f "${PROJECT_ROOT}/.env.example" ]]; then
  install -m 0644 "${PROJECT_ROOT}/.env.example" "${RUNTIME_SOURCE_DIR}/.env.example"
fi

echo "[opentyless-npm] staged runtime into ${RUNTIME_SOURCE_DIR}"

#!/usr/bin/env bash
set -u

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
if command -v opentyless-rs >/dev/null 2>&1; then
  BIN_PATH="$(command -v opentyless-rs)"
elif [[ -x "${SCRIPT_DIR}/opentyless-rs" ]]; then
  BIN_PATH="${SCRIPT_DIR}/opentyless-rs"
elif [[ -x "${SCRIPT_DIR}/../target/release/opentyless-rs" ]]; then
  BIN_PATH="${SCRIPT_DIR}/../target/release/opentyless-rs"
else
  BIN_PATH=""
fi

STATE_HOME="${XDG_STATE_HOME:-${HOME}/.local/state}"
LOG_DIR="${STATE_HOME}/opentyless"
LOG_PATH="${LOG_DIR}/hotkey.log"
MAX_LOG_BYTES=262144

mkdir -p "${LOG_DIR}"

if [[ -f "${LOG_PATH}" ]]; then
  log_size="$(wc -c <"${LOG_PATH}" 2>/dev/null || echo 0)"
  if [[ "${log_size}" -gt "${MAX_LOG_BYTES}" ]]; then
    tail -n 200 "${LOG_PATH}" >"${LOG_PATH}.tmp" 2>/dev/null && mv -f "${LOG_PATH}.tmp" "${LOG_PATH}"
  fi
fi

timestamp() {
  date '+%Y-%m-%d %H:%M:%S%z'
}

{
  echo "=== hotkey trigger $(timestamp) ==="
  echo "cwd: $(pwd)"
  echo "whoami: $(whoami)"
  echo "DISPLAY: ${DISPLAY:-}"
  echo "WAYLAND_DISPLAY: ${WAYLAND_DISPLAY:-}"
  echo "DBUS_SESSION_BUS_ADDRESS: ${DBUS_SESSION_BUS_ADDRESS:-}"
  echo "bin_path: ${BIN_PATH}"

  if [[ -z "${BIN_PATH}" || ! -x "${BIN_PATH}" ]]; then
    echo "error: binary not executable: ${BIN_PATH}"
    exit 127
  fi

  "${BIN_PATH}" toggle-record
  rc=$?
  echo "exit_code: ${rc}"
  echo
  exit "${rc}"
} >>"${LOG_PATH}" 2>&1

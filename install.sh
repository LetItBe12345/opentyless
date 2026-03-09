#!/usr/bin/env bash
set -euo pipefail

PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
INSTALL_BIN_DIR="${HOME}/.local/bin"
INSTALL_BIN_PATH="${INSTALL_BIN_DIR}/opentyless-rs"
HOTKEY_WRAPPER_PATH="${INSTALL_BIN_DIR}/opentyless-hotkey-toggle"
ENABLE_SERVICE=1
INSTALL_TRAY=1
INSTALL_SHORTCUT=0
SHORTCUT_BINDING="F8"
SKIP_DEPS=0

log() {
  printf '[opentyless-install] %s\n' "$*"
}

warn() {
  printf '[opentyless-install] 警告: %s\n' "$*" >&2
}

die() {
  printf '[opentyless-install] 错误: %s\n' "$*" >&2
  exit 1
}

usage() {
  cat <<'EOF'
用法:
  ./install.sh [选项]

选项:
  --skip-deps           跳过系统依赖安装
  --no-service          不安装 systemd --user 服务
  --no-tray             不安装 GNOME 托盘自启动
  --install-shortcut    安装 GNOME 快捷键
  --binding <keys>      GNOME 快捷键绑定，默认 F8
  -h, --help            显示帮助

说明:
  1. 自动检查并安装 Rust（通过 rustup）
  2. 在 Debian/Ubuntu 上自动安装常用系统依赖
  3. 编译 release 二进制并安装到 ~/.local/bin/opentyless-rs
  4. 安装热键包装脚本 ~/.local/bin/opentyless-hotkey-toggle
  5. 如不存在 .env，则从 .env.example 复制
  6. 默认安装并启用 systemd 用户服务，以及 GNOME 托盘自启动
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --skip-deps)
      SKIP_DEPS=1
      shift
      ;;
    --no-service)
      ENABLE_SERVICE=0
      shift
      ;;
    --no-tray)
      INSTALL_TRAY=0
      shift
      ;;
    --install-shortcut)
      INSTALL_SHORTCUT=1
      shift
      ;;
    --binding)
      [[ $# -ge 2 ]] || die "--binding 需要一个参数"
      SHORTCUT_BINDING="$2"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      die "未知参数: $1"
      ;;
  esac
done

install_apt_deps() {
  local packages=(
    build-essential
    pkg-config
    curl
    git
    ffmpeg
    xclip
    wl-clipboard
    libasound2-dev
    libdbus-1-dev
  )

  if ! command -v apt-get >/dev/null 2>&1; then
    warn "未检测到 apt-get，跳过系统依赖自动安装。请自行确保已安装: ${packages[*]}"
    return
  fi

  log "安装系统依赖: ${packages[*]}"
  sudo apt-get update
  sudo apt-get install -y "${packages[@]}"
}

ensure_rust() {
  if command -v cargo >/dev/null 2>&1 && command -v rustc >/dev/null 2>&1; then
    return
  fi

  log "未检测到 Rust，开始安装 rustup"
  curl --proto '=https' --tlsv1.2 -fsSL https://sh.rustup.rs | sh -s -- -y
  if [[ -f "${HOME}/.cargo/env" ]]; then
    # shellcheck disable=SC1090
    source "${HOME}/.cargo/env"
  fi
}

ensure_path_hint() {
  case ":${PATH}:" in
    *":${INSTALL_BIN_DIR}:"*) ;;
    *)
      warn "~/.local/bin 当前不在 PATH 中。建议把下面这行加入 ~/.bashrc 或 ~/.zshrc:"
      warn 'export PATH="$HOME/.local/bin:$PATH"'
      ;;
  esac
}

prepare_env_file() {
  if [[ -f "${PROJECT_ROOT}/.env" ]]; then
    return
  fi
  cp "${PROJECT_ROOT}/.env.example" "${PROJECT_ROOT}/.env"
  warn "已生成 .env，请至少填写 DASHSCOPE_API_KEY 后再正式使用。"
}

build_release() {
  log "编译 release 版本"
  cargo build --release --manifest-path "${PROJECT_ROOT}/Cargo.toml"
}

install_binary() {
  mkdir -p "${INSTALL_BIN_DIR}"
  cp "${PROJECT_ROOT}/target/release/opentyless-rs" "${INSTALL_BIN_PATH}"
  chmod +x "${INSTALL_BIN_PATH}"
  log "已安装二进制: ${INSTALL_BIN_PATH}"
}

install_hotkey_wrapper() {
  mkdir -p "${INSTALL_BIN_DIR}"
  cp "${PROJECT_ROOT}/scripts/hotkey-toggle.sh" "${HOTKEY_WRAPPER_PATH}"
  chmod +x "${HOTKEY_WRAPPER_PATH}"
  log "已安装热键包装脚本: ${HOTKEY_WRAPPER_PATH}"
}

run_doctor() {
  log "运行环境检查"
  "${INSTALL_BIN_PATH}" doctor || true
}

install_service() {
  log "安装并启用 systemd 用户服务"
  (cd "${PROJECT_ROOT}" && "${INSTALL_BIN_PATH}" install-service --enable)
}

install_tray() {
  log "安装 GNOME 托盘自启动"
  (cd "${PROJECT_ROOT}" && "${INSTALL_BIN_PATH}" install-tray-autostart)
}

install_shortcut() {
  log "安装 GNOME 快捷键: ${SHORTCUT_BINDING}"
  (cd "${PROJECT_ROOT}" && "${INSTALL_BIN_PATH}" install-gnome-shortcut --binding "${SHORTCUT_BINDING}")
}

main() {
  if [[ ${SKIP_DEPS} -eq 0 ]]; then
    install_apt_deps
  fi

  ensure_rust
  prepare_env_file
  build_release
  install_binary
  install_hotkey_wrapper
  ensure_path_hint
  run_doctor

  if [[ ${ENABLE_SERVICE} -eq 1 ]]; then
    install_service
  fi

  if [[ ${INSTALL_TRAY} -eq 1 ]]; then
    install_tray
  fi

  if [[ ${INSTALL_SHORTCUT} -eq 1 ]]; then
    install_shortcut
  fi

  log "安装完成"
  log "如需立即测试，可执行: ${INSTALL_BIN_PATH} status"
  log "首次使用前请确认 ${PROJECT_ROOT}/.env 中已填写 DASHSCOPE_API_KEY"
}

main "$@"

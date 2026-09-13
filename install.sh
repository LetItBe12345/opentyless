#!/usr/bin/env bash
set -euo pipefail

PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
INSTALL_BIN_DIR="${HOME}/.local/bin"
INSTALL_BIN_PATH="${INSTALL_BIN_DIR}/opentyless-rs"
HOTKEY_WRAPPER_PATH="${INSTALL_BIN_DIR}/opentyless-hotkey-toggle"
SESSION_TYPE="${XDG_SESSION_TYPE:-unknown}"
CURRENT_DESKTOP="${XDG_CURRENT_DESKTOP:-${DESKTOP_SESSION:-unknown}}"
ENABLE_SERVICE=1
INSTALL_TRAY=1
INSTALL_SHORTCUT=0
UNINSTALL_SHORTCUT=0
SHORTCUT_BINDING=""
FORCE_SHORTCUT=0
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
  --uninstall-shortcut  删除由 OpenTyless 管理的桌面快捷键后退出
  --binding <keys>      快捷键绑定；Omarchy 默认 SUPER + V，GNOME 默认 <Super>v
  --force               即使快捷键已被占用也覆盖（仅 Omarchy/Hyprland）
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
    --uninstall-shortcut)
      UNINSTALL_SHORTCUT=1
      shift
      ;;
    --force)
      FORCE_SHORTCUT=1
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
    command -v pacman >/dev/null 2>&1 && return
    warn "未检测到 apt-get，跳过系统依赖自动安装。请自行确保已安装: ${packages[*]}"
    return
  fi

  log "安装系统依赖: ${packages[*]}"
  sudo apt-get update
  sudo apt-get install -y "${packages[@]}"
}

install_arch_deps() {
  command -v pacman >/dev/null 2>&1 || return

  local packages=(base-devel pkgconf ffmpeg wl-clipboard alsa-lib dbus libayatana-appindicator)
  local missing=()
  local package
  for package in "${packages[@]}"; do
    pacman -Q "${package}" >/dev/null 2>&1 || missing+=("${package}")
  done
  if [[ ${#missing[@]} -eq 0 ]]; then
    return
  fi
  command -v omarchy >/dev/null 2>&1 || die "缺少 Arch 依赖: ${missing[*]}；未找到 omarchy 命令"
  log "通过 omarchy 安装缺失依赖: ${missing[*]}"
  omarchy pkg add "${missing[@]}"
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

read_env_value() {
  local key="$1"
  local env_file="${PROJECT_ROOT}/.env"
  if [[ ! -f "${env_file}" ]]; then
    return
  fi
  sed -n "s/^${key}=//p" "${env_file}" | tail -n 1
}

normalize_env_value() {
  local value="${1:-}"
  value="${value#\"}"
  value="${value%\"}"
  printf '%s' "${value}"
}

upsert_env_value() {
  local key="$1"
  local value="$2"
  local env_file="${PROJECT_ROOT}/.env"
  local tmp_file
  tmp_file="$(mktemp)"
  awk -v key="${key}" -v value="${value}" '
    BEGIN { replaced = 0 }
    index($0, key "=") == 1 {
      print key "=" value
      replaced = 1
      next
    }
    { print }
    END {
      if (!replaced) {
        print key "=" value
      }
    }
  ' "${env_file}" > "${tmp_file}"
  mv "${tmp_file}" "${env_file}"
}

prepare_env_file() {
  if [[ -f "${PROJECT_ROOT}/.env" ]]; then
    return
  fi
  cp "${PROJECT_ROOT}/.env.example" "${PROJECT_ROOT}/.env"
  warn "已生成 .env，请至少填写 DASHSCOPE_API_KEY 后再正式使用。"
  chmod 600 "${PROJECT_ROOT}/.env"
}

secure_env_file() {
  [[ -f "${PROJECT_ROOT}/.env" ]] || return
  chmod 600 "${PROJECT_ROOT}/.env"
}

configure_session_env_defaults() {
  local current_clipboard
  local clipboard_bin=""

  current_clipboard="$(normalize_env_value "$(read_env_value CLIPBOARD_COMMAND)")"
  if [[ -n "${current_clipboard}" ]]; then
    return
  fi

  case "${SESSION_TYPE}" in
    x11)
      clipboard_bin="$(command -v xclip || true)"
      if [[ -n "${clipboard_bin}" ]]; then
        upsert_env_value "CLIPBOARD_COMMAND" "\"${clipboard_bin} -selection clipboard\""
        log "检测到 X11，会默认使用 xclip 作为剪贴板命令"
      else
        warn "当前是 X11，但未找到 xclip；请安装后再重新运行 install.sh，或手动设置 CLIPBOARD_COMMAND"
      fi
      ;;
    wayland)
      clipboard_bin="$(command -v wl-copy || true)"
      if [[ -n "${clipboard_bin}" ]]; then
        upsert_env_value "CLIPBOARD_COMMAND" "\"${clipboard_bin}\""
        log "检测到 Wayland，会默认使用 wl-copy 作为剪贴板命令"
      fi
      ;;
  esac
}

build_release() {
  log "编译 release 版本"
  cargo build --release --manifest-path "${PROJECT_ROOT}/Cargo.toml"
}

install_binary() {
  mkdir -p "${INSTALL_BIN_DIR}"
  local staged_binary
  staged_binary="$(mktemp "${INSTALL_BIN_DIR}/.opentyless-rs.XXXXXX")"
  cp "${PROJECT_ROOT}/target/release/opentyless-rs" "${staged_binary}"
  chmod +x "${staged_binary}"
  mv -f "${staged_binary}" "${INSTALL_BIN_PATH}"
  log "已安装二进制: ${INSTALL_BIN_PATH}"
}

install_hotkey_wrapper() {
  mkdir -p "${INSTALL_BIN_DIR}"
  local staged_wrapper
  staged_wrapper="$(mktemp "${INSTALL_BIN_DIR}/.opentyless-hotkey-toggle.XXXXXX")"
  cp "${PROJECT_ROOT}/scripts/hotkey-toggle.sh" "${staged_wrapper}"
  chmod +x "${staged_wrapper}"
  mv -f "${staged_wrapper}" "${HOTKEY_WRAPPER_PATH}"
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
  case "${CURRENT_DESKTOP}" in
    *Hyprland*|*hyprland*)
      log "安装并启用 Hyprland/Omarchy 托盘用户服务"
      (cd "${PROJECT_ROOT}" && "${INSTALL_BIN_PATH}" install-tray-service --enable)
      ;;
    *)
      log "安装桌面托盘自启动"
      (cd "${PROJECT_ROOT}" && "${INSTALL_BIN_PATH}" install-tray-autostart)
      ;;
  esac
}

start_tray_now() {
  if [[ "${INSTALL_TRAY}" -ne 1 ]]; then
    return
  fi

  case "${CURRENT_DESKTOP}" in
    *GNOME*|*gnome*|*ubuntu*|*Ubuntu*) ;;
    *) return ;;
  esac

  if [[ -z "${DISPLAY:-}" && -z "${WAYLAND_DISPLAY:-}" ]]; then
    return
  fi

  if pgrep -f "^${INSTALL_BIN_PATH} tray$" >/dev/null 2>&1; then
    return
  fi

  if setsid -f "${INSTALL_BIN_PATH}" tray >/tmp/opentyless-tray.log 2>&1; then
    log "已在当前桌面会话启动托盘"
  else
    warn "托盘自启动已安装，但当前会话即时启动失败；可手动执行: ${INSTALL_BIN_PATH} tray"
  fi
}

install_shortcut() {
  case "${CURRENT_DESKTOP}" in
    *Hyprland*|*hyprland*) install_hyprland_shortcut ;;
    *)
      log "安装 GNOME 快捷键: ${SHORTCUT_BINDING}"
      (cd "${PROJECT_ROOT}" && "${INSTALL_BIN_PATH}" install-gnome-shortcut --binding "${SHORTCUT_BINDING}")
      ;;
  esac
}

hyprland_bindings_path() {
  printf '%s' "${HOME}/.config/hypr/bindings.lua"
}

remove_managed_hyprland_block() {
  local bindings_path="$1"
  local tmp_file
  tmp_file="$(mktemp)"
  awk '
    /^-- BEGIN OPENTYLESS MANAGED SHORTCUT$/ { managed = 1; next }
    /^-- END OPENTYLESS MANAGED SHORTCUT$/ { managed = 0; next }
    !managed { print }
  ' "${bindings_path}" > "${tmp_file}"
  mv "${tmp_file}" "${bindings_path}"
}

backup_hyprland_bindings() {
  local bindings_path="$1"
  local backup_path="${bindings_path}.opentyless.bak.$(date +%Y%m%d%H%M%S)"
  cp -- "${bindings_path}" "${backup_path}"
  log "已备份 Hyprland 快捷键配置: ${backup_path}"
}

install_hyprland_shortcut() {
  local bindings_path
  bindings_path="$(hyprland_bindings_path)"
  [[ -f "${bindings_path}" ]] || die "未找到 Hyprland 快捷键配置: ${bindings_path}"
  [[ "${SHORTCUT_BINDING}" =~ ^[A-Za-z0-9_+[:space:]-]+$ ]] || die "不支持的 Hyprland 快捷键格式: ${SHORTCUT_BINDING}"

  local already_managed=0
  grep -q '^-- BEGIN OPENTYLESS MANAGED SHORTCUT$' "${bindings_path}" && already_managed=1
  if [[ ${already_managed} -eq 0 ]] && command -v omarchy >/dev/null 2>&1; then
    if omarchy menu keybindings --print 2>/dev/null | awk -F '→' -v wanted="${SHORTCUT_BINDING}" '
      { key=$1; gsub(/^[[:space:]]+|[[:space:]]+$/, "", key); if (toupper(key) == toupper(wanted)) found=1 }
      END { exit(found ? 0 : 1) }
    '; then
      [[ ${FORCE_SHORTCUT} -eq 1 ]] || die "快捷键 ${SHORTCUT_BINDING} 已被占用；请更换 --binding，或确认后使用 --force"
      warn "将覆盖已经存在的快捷键: ${SHORTCUT_BINDING}"
    fi
  fi

  backup_hyprland_bindings "${bindings_path}"
  remove_managed_hyprland_block "${bindings_path}"
  {
    printf '\n-- BEGIN OPENTYLESS MANAGED SHORTCUT\n'
    printf 'hl.unbind("%s")\n' "${SHORTCUT_BINDING}"
    printf 'o.bind("%s", "OpenTyless voice input", "%s")\n' "${SHORTCUT_BINDING}" "${HOTKEY_WRAPPER_PATH}"
    printf '%s\n' '-- END OPENTYLESS MANAGED SHORTCUT'
  } >> "${bindings_path}"
  log "已安装 Hyprland 快捷键: ${SHORTCUT_BINDING} -> ${HOTKEY_WRAPPER_PATH}"
}

uninstall_hyprland_shortcut() {
  local bindings_path
  bindings_path="$(hyprland_bindings_path)"
  [[ -f "${bindings_path}" ]] || die "未找到 Hyprland 快捷键配置: ${bindings_path}"
  if ! grep -q '^-- BEGIN OPENTYLESS MANAGED SHORTCUT$' "${bindings_path}"; then
    log "未发现由 OpenTyless 管理的 Hyprland 快捷键"
    return
  fi
  backup_hyprland_bindings "${bindings_path}"
  remove_managed_hyprland_block "${bindings_path}"
  log "已移除 OpenTyless Hyprland 快捷键"
}

main() {
  log "检测到桌面会话: session=${SESSION_TYPE}, desktop=${CURRENT_DESKTOP}"

  if [[ -z "${SHORTCUT_BINDING}" ]]; then
    case "${CURRENT_DESKTOP}" in
      *Hyprland*|*hyprland*) SHORTCUT_BINDING="SUPER + V" ;;
      *) SHORTCUT_BINDING="<Super>v" ;;
    esac
  fi

  if [[ ${UNINSTALL_SHORTCUT} -eq 1 ]]; then
    case "${CURRENT_DESKTOP}" in
      *Hyprland*|*hyprland*) uninstall_hyprland_shortcut ;;
      *) die "自动卸载快捷键目前仅支持 Hyprland/Omarchy" ;;
    esac
    exit 0
  fi

  if [[ ${SKIP_DEPS} -eq 0 ]]; then
    install_apt_deps
    install_arch_deps
  fi

  ensure_rust
  prepare_env_file
  secure_env_file
  configure_session_env_defaults
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
    start_tray_now
  fi

  if [[ ${INSTALL_SHORTCUT} -eq 1 ]]; then
    install_shortcut
  fi

  log "安装完成"
  log "如需立即测试，可执行: ${INSTALL_BIN_PATH} status"
  log "首次使用前请确认 ${PROJECT_ROOT}/.env 中已填写 DASHSCOPE_API_KEY"
}

main "$@"

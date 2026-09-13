#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
INSTALL="${ROOT}/install.sh"
TMP="$(mktemp -d)"
trap 'rm -rf "${TMP}"' EXIT

export HOME="${TMP}"
export XDG_CURRENT_DESKTOP=Hyprland

mkdir -p "${HOME}/.config/hypr" "${HOME}/.local/bin" "${TMP}/bin"
cat > "${TMP}/bin/omarchy" <<'EOF'
#!/bin/sh
exit 1
EOF
chmod +x "${TMP}/bin/omarchy"
export PATH="${TMP}/bin:/usr/bin:/bin:/usr/local/bin"
cat > "${HOME}/.config/hypr/bindings.lua" <<'EOF'
-- user config
o.bind("SUPER + Q", "Quit", "true")
EOF

extract_fn() {
  local name="$1"
  awk -v name="$name" '
    $0 ~ "^" name "\\(\\) \\{" { printing = 1 }
    printing { print }
    printing && $0 == "}" { exit }
  ' "${INSTALL}"
}

eval "$(extract_fn log)"
eval "$(extract_fn warn)"
eval "$(extract_fn die)"
eval "$(extract_fn hyprland_bindings_path)"
eval "$(extract_fn remove_managed_hyprland_block)"
eval "$(extract_fn backup_hyprland_bindings)"
eval "$(extract_fn install_hyprland_shortcut)"
eval "$(extract_fn uninstall_hyprland_shortcut)"

HOTKEY_WRAPPER_PATH="${HOME}/.local/bin/opentyless-hotkey-toggle"
SHORTCUT_BINDING="SUPER + V"
FORCE_SHORTCUT=0
touch "${HOTKEY_WRAPPER_PATH}"

install_hyprland_shortcut
grep -q '^-- BEGIN OPENTYLESS MANAGED SHORTCUT$' "${HOME}/.config/hypr/bindings.lua"
grep -q 'hl.unbind("SUPER + V")' "${HOME}/.config/hypr/bindings.lua"
grep -q 'o.bind("SUPER + V", "OpenTyless voice input"' "${HOME}/.config/hypr/bindings.lua"
grep -q 'o.bind("SUPER + Q", "Quit", "true")' "${HOME}/.config/hypr/bindings.lua"
compgen -G "${HOME}/.config/hypr/bindings.lua.opentyless.bak.*" >/dev/null

install_hyprland_shortcut
count="$(grep -c '^-- BEGIN OPENTYLESS MANAGED SHORTCUT$' "${HOME}/.config/hypr/bindings.lua")"
[[ "${count}" -eq 1 ]]

if ( SHORTCUT_BINDING='bad;binding' install_hyprland_shortcut ); then
  echo "invalid binding should be rejected" >&2
  exit 1
fi

SHORTCUT_BINDING="SUPER + V"
uninstall_hyprland_shortcut
if grep -q '^-- BEGIN OPENTYLESS MANAGED SHORTCUT$' "${HOME}/.config/hypr/bindings.lua"; then
  echo "managed block should be removed" >&2
  exit 1
fi
grep -q 'o.bind("SUPER + Q", "Quit", "true")' "${HOME}/.config/hypr/bindings.lua"

uninstall_hyprland_shortcut

echo "hyprland shortcut e2e ok"

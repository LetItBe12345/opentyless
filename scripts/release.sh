#!/usr/bin/env bash
set -euo pipefail

PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TAG=""
TITLE=""
PUSH_FIRST=0
SKIP_BUILD=0
TRACK=""

log() {
  printf '[opentyless-release] %s\n' "$*"
}

warn() {
  printf '[opentyless-release] warning: %s\n' "$*" >&2
}

die() {
  printf '[opentyless-release] error: %s\n' "$*" >&2
  exit 1
}

usage() {
  cat <<'EOF'
Usage:
  ./scripts/release.sh --tag <tag> [options]

Options:
  --tag <tag>        Git tag / GitHub release tag
  --title <title>    Optional release title, defaults to tag
  --push-first       Push current branch before creating the release
  --skip-build       Reuse existing target/release binary
  --track <name>     Deprecated compatibility flag; ignored
  -h, --help         Show help

Examples:
  ./scripts/release.sh --tag v0.2.0 --push-first
  ./scripts/release.sh --tag v0.2.1
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --tag)
      [[ $# -ge 2 ]] || die "--tag requires a value"
      TAG="$2"
      shift 2
      ;;
    --title)
      [[ $# -ge 2 ]] || die "--title requires a value"
      TITLE="$2"
      shift 2
      ;;
    --push-first)
      PUSH_FIRST=1
      shift
      ;;
    --skip-build)
      SKIP_BUILD=1
      shift
      ;;
    --track)
      [[ $# -ge 2 ]] || die "--track requires a value"
      TRACK="$2"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      die "unknown argument: $1"
      ;;
  esac
done

[[ -n "${TAG}" ]] || die "--tag is required"

if [[ -n "${TRACK}" ]]; then
  warn "--track 已废弃，当前发布流程统一为单发布线；收到值: ${TRACK}"
fi

if [[ -z "${TITLE}" ]]; then
  TITLE="${TAG}"
fi

require_command() {
  command -v "$1" >/dev/null 2>&1 || die "missing required command: $1"
}

require_command git
require_command tar
require_command sha256sum
require_command gh
require_command cargo

CURRENT_BRANCH="$(git -C "${PROJECT_ROOT}" branch --show-current)"

if [[ -n "$(git -C "${PROJECT_ROOT}" status --short --untracked-files=no)" ]]; then
  die "working tree has tracked changes; commit or stash them first"
fi

if [[ ${PUSH_FIRST} -eq 1 ]]; then
  log "pushing branch ${CURRENT_BRANCH}"
  git -C "${PROJECT_ROOT}" push origin "${CURRENT_BRANCH}"
fi

if gh release view "${TAG}" >/dev/null 2>&1; then
  die "release tag already exists: ${TAG}"
fi

if [[ ${SKIP_BUILD} -eq 0 ]]; then
  log "building release binary"
  cargo build --release --manifest-path "${PROJECT_ROOT}/Cargo.toml"
fi

VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' "${PROJECT_ROOT}/Cargo.toml" | head -n 1)"
[[ -n "${VERSION}" ]] || die "failed to read version from Cargo.toml"

PACKAGE_SUFFIX="linux-x86_64"
PACKAGE_BASENAME="opentyless-rs-${TAG}-${PACKAGE_SUFFIX}"
DIST_DIR="${PROJECT_ROOT}/dist"
PACKAGE_DIR="${DIST_DIR}/${PACKAGE_BASENAME}"
ARCHIVE_PATH="${DIST_DIR}/${PACKAGE_BASENAME}.tar.gz"
CHECKSUM_PATH="${DIST_DIR}/SHA256SUMS.txt"

log "preparing dist directory"
rm -rf "${PACKAGE_DIR}"
mkdir -p "${PACKAGE_DIR}"
install -m 0755 "${PROJECT_ROOT}/target/release/opentyless-rs" "${PACKAGE_DIR}/opentyless-rs"
install -m 0755 "${PROJECT_ROOT}/scripts/hotkey-toggle.sh" "${PACKAGE_DIR}/opentyless-hotkey-toggle"
install -m 0644 "${PROJECT_ROOT}/README.md" "${PACKAGE_DIR}/README.md"
install -m 0644 "${PROJECT_ROOT}/README.zh-CN.md" "${PACKAGE_DIR}/README.zh-CN.md"
install -m 0644 "${PROJECT_ROOT}/.env.example" "${PACKAGE_DIR}/.env.example"

log "creating archive ${ARCHIVE_PATH}"
tar -C "${DIST_DIR}" -czf "${ARCHIVE_PATH}" "${PACKAGE_BASENAME}"
sha256sum "${ARCHIVE_PATH}" > "${CHECKSUM_PATH}"

RELEASE_NOTES="$(mktemp)"
cat > "${RELEASE_NOTES}" <<EOF
Branch: ${CURRENT_BRANCH}
Version: ${VERSION}

Artifacts:
- $(basename "${ARCHIVE_PATH}")
- $(basename "${CHECKSUM_PATH}")

Desktop integration:
- Runtime chooses clipboard defaults from XDG_SESSION_TYPE
- install.sh also auto-tunes clipboard/tray defaults from the desktop session

Documentation:
- English: README.md
- Simplified Chinese: README.zh-CN.md
EOF

log "creating GitHub release ${TAG}"
gh release create "${TAG}" \
  "${ARCHIVE_PATH}" \
  "${CHECKSUM_PATH}" \
  --target "${CURRENT_BRANCH}" \
  --title "${TITLE}" \
  --notes-file "${RELEASE_NOTES}"

rm -f "${RELEASE_NOTES}"
log "release created successfully"

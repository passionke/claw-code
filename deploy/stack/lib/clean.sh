#!/usr/bin/env bash
# Remove local Rust/build artifacts under the repo (not podman images/volumes). Author: kejiqing
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
RUST_DIR="${ROOT_DIR}/rust"
LINUX_ARTIFACTS="${ROOT_DIR}/deploy/stack/.linux-artifacts"
CLAW_WORKSPACE="${ROOT_DIR}/deploy/stack/claw-workspace"

clean_usage() {
  cat <<EOF
Usage: gateway.sh clean [options]

Removes:
  - rust/target/ (cargo clean)
  - deploy/stack/.linux-artifacts/ (Darwin podman-run compile output)

Options:
  --workspace   Also remove deploy/stack/claw-workspace/ (ds sessions; destructive)
  -h, --help    Show this help
EOF
}

claw_dir_size() {
  local p="$1"
  if [[ -e "${p}" ]]; then
    du -sh "${p}" 2>/dev/null | awk '{print $1}'
  else
    printf '%s' "0"
  fi
}

CLAW_CLEAN_WORKSPACE=0
while [[ $# -gt 0 ]]; do
  case "$1" in
    -h | --help)
      clean_usage
      exit 0
      ;;
    --workspace)
      CLAW_CLEAN_WORKSPACE=1
      shift
      ;;
    --*)
      echo "unknown clean option: $1" >&2
      clean_usage >&2
      exit 2
      ;;
    *)
      echo "unexpected argument: $1" >&2
      clean_usage >&2
      exit 2
      ;;
  esac
done

echo "==> gateway clean (build artifacts only)"
before_target="$(claw_dir_size "${RUST_DIR}/target")"
before_linux="$(claw_dir_size "${LINUX_ARTIFACTS}")"
before_ws="$(claw_dir_size "${CLAW_WORKSPACE}")"

if [[ -d "${RUST_DIR}/target" ]]; then
  if command -v cargo >/dev/null 2>&1; then
    (cd "${RUST_DIR}" && cargo clean)
  else
    rm -rf "${RUST_DIR}/target"
    echo "    removed ${RUST_DIR}/target (no cargo in PATH)"
  fi
else
  echo "    rust/target: already absent"
fi

if [[ -d "${LINUX_ARTIFACTS}" ]]; then
  rm -rf "${LINUX_ARTIFACTS}"
  echo "    removed ${LINUX_ARTIFACTS}"
else
  echo "    .linux-artifacts: already absent"
fi

if [[ "${CLAW_CLEAN_WORKSPACE}" == "1" ]]; then
  if [[ -d "${CLAW_WORKSPACE}" ]]; then
    rm -rf "${CLAW_WORKSPACE}"
    echo "    removed ${CLAW_WORKSPACE}"
  else
    echo "    claw-workspace: already absent"
  fi
fi

after_total="$(du -sh "${ROOT_DIR}" 2>/dev/null | awk '{print $1}')"
echo "==> freed (was): rust/target=${before_target} .linux-artifacts=${before_linux} workspace=${before_ws}"
echo "==> repo total now: ${after_total}"
echo "    (podman images/volumes unchanged; use: podman system prune)"

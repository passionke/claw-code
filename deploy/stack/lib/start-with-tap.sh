#!/usr/bin/env bash
set -euo pipefail

LIB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PODMAN_DIR="$(cd "${LIB_DIR}/.." && pwd)"
ROOT_DIR="$(cd "${PODMAN_DIR}/../.." && pwd)"
ROOT_ENV="${ROOT_DIR}/.env"

if [[ ! -f "${ROOT_ENV}" ]]; then
  echo "missing ${ROOT_ENV}" >&2
  echo "copy ${ROOT_DIR}/.env.example to ${ROOT_ENV} and edit" >&2
  exit 1
fi

set -a
# shellcheck disable=SC1090
source "${ROOT_ENV}"
set +a

# shellcheck source=/dev/null
source "${LIB_DIR}/claude-tap-local.sh"
claw_claude_tap_start "${PODMAN_DIR}" "${ROOT_DIR}"

# Compose bind-mounts repo-root `.claw.json`. Never overwrite an existing file — only create `{}` if missing. kejiqing
CLAW_JSON="${ROOT_DIR}/.claw.json"
if [[ ! -f "${CLAW_JSON}" ]]; then
  echo "note: ${CLAW_JSON} missing; creating empty {} stub (existing files are never touched)." >&2
  printf '%s\n' '{}' >"${CLAW_JSON}"
fi

# shellcheck disable=SC1090
source "${PODMAN_DIR}/lib/compose-include.sh"
claw_podman_export_pool_workspace "${PODMAN_DIR}"
claw_podman_load_compose_args "${PODMAN_DIR}" "${ROOT_ENV}"

install_args=()
if [[ -n "${CLAW_IMAGE_RELEASE_TAG:-}" ]]; then
  install_args+=("--release" "${CLAW_IMAGE_RELEASE_TAG}")
fi
install_args+=("${CLAW_POOL_DAEMON_BIN:-${ROOT_DIR}/rust/target/release/claw-pool-daemon}")
"${PODMAN_DIR}/lib/install-pool-daemon-from-image.sh" "${install_args[@]}"
"${PODMAN_DIR}/lib/pool-daemon-up.sh" "${PODMAN_DIR}" "${ROOT_DIR}"

claw_compose_with_root_env "${PODMAN_DIR}" "${ROOT_ENV}" "${CLAW_PODMAN_COMPOSE_ARGS[@]}" up -d --force-recreate
echo "gateway started on port ${GATEWAY_HOST_PORT}"
echo "claude-tap mode=${CLAUDE_TAP_MODE:-docker} live viewer: http://127.0.0.1:${CLAUDE_TAP_LIVE_PORT:-3000}"

#!/usr/bin/env bash
set -euo pipefail

LIB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PODMAN_DIR="$(cd "${LIB_DIR}/.." && pwd)"
REPO_ROOT="$(cd "${PODMAN_DIR}/../.." && pwd)"
ROOT_ENV="${REPO_ROOT}/.env"

if [[ -f "${ROOT_ENV}" ]]; then
  set -a
  # shellcheck disable=SC1090
  source "${ROOT_ENV}"
  set +a
  # shellcheck disable=SC1090
  source "${PODMAN_DIR}/lib/compose-include.sh"
  claw_podman_export_pool_workspace "${PODMAN_DIR}"
  claw_podman_load_compose_args "${PODMAN_DIR}" "${ROOT_ENV}"
  claw_compose --env-file "${ROOT_ENV}" "${CLAW_PODMAN_COMPOSE_ARGS[@]}" down
  "${PODMAN_DIR}/lib/pool-daemon-down.sh"
else
  # shellcheck disable=SC1090
  source "${PODMAN_DIR}/lib/compose-include.sh"
  claw_compose -f "${PODMAN_DIR}/podman-compose.yml" down
  "${PODMAN_DIR}/lib/pool-daemon-down.sh"
fi

# shellcheck source=/dev/null
source "${LIB_DIR}/claude-tap-local.sh"
claw_claude_tap_stop "${PODMAN_DIR}"

echo "gateway stopped, claude-tap stopped"

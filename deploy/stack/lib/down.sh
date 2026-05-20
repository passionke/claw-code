#!/usr/bin/env bash
set -euo pipefail

LIB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PODMAN_DIR="$(cd "${LIB_DIR}/.." && pwd)"
REPO_ROOT="$(cd "${PODMAN_DIR}/../.." && pwd)"
ENV_FILE="${REPO_ROOT}/.env"

if [[ -f "${ENV_FILE}" ]]; then
  # shellcheck disable=SC1090
  source "${PODMAN_DIR}/lib/compose-include.sh"
  claw_podman_export_pool_workspace "${PODMAN_DIR}"
  claw_podman_load_compose_args "${PODMAN_DIR}" "${ENV_FILE}"
  claw_compose_gateway_down "${PODMAN_DIR}" "${ENV_FILE}"
  "${PODMAN_DIR}/lib/pool-daemon-down.sh"
else
  # podman-compose.yml references ./.claw-pool-workspace.env; create a stub so compose works.
  # shellcheck disable=SC1090
  source "${PODMAN_DIR}/lib/compose-include.sh"
  claw_podman_export_pool_workspace "${PODMAN_DIR}"
  claw_compose -f "${PODMAN_DIR}/podman-compose.yml" stop gateway-rs 2>/dev/null || true
  claw_compose -f "${PODMAN_DIR}/podman-compose.yml" rm -f gateway-rs 2>/dev/null || true
  "${PODMAN_DIR}/lib/pool-daemon-down.sh"
fi

echo "Gateway stack stopped (postgres unchanged; use gateway.sh pg-down to stop PG)."

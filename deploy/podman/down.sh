#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
ENV_FILE="${REPO_ROOT}/.env"

if [[ -f "${ENV_FILE}" ]]; then
  # shellcheck disable=SC1090
  source "${SCRIPT_DIR}/compose-include.sh"
  claw_podman_export_pool_workspace "${SCRIPT_DIR}"
  claw_podman_load_compose_args "${SCRIPT_DIR}" "${ENV_FILE}"
  podman compose --env-file "${ENV_FILE}" "${CLAW_PODMAN_COMPOSE_ARGS[@]}" down
else
  podman compose -f "${SCRIPT_DIR}/podman-compose.yml" down
fi

echo "Services stopped."

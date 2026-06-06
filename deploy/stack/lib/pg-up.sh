#!/usr/bin/env bash
# Start Postgres only (gateway stack data). Author: kejiqing
set -euo pipefail

LIB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PODMAN_DIR="$(cd "${LIB_DIR}/.." && pwd)"
REPO_ROOT="$(cd "${PODMAN_DIR}/../.." && pwd)"
ENV_FILE="${REPO_ROOT}/.env"

# shellcheck disable=SC1090
source "${PODMAN_DIR}/lib/compose-include.sh"
claw_podman_export_pool_workspace "${PODMAN_DIR}"

if [[ ! -f "${ENV_FILE}" ]]; then
  echo "missing ${ENV_FILE}" >&2
  exit 1
fi

claw_podman_load_compose_args "${PODMAN_DIR}" "${ENV_FILE}"
claw_compose_pg_ensure "${PODMAN_DIR}" "${ENV_FILE}"
claw_compose_pg_wait_healthy
pg="$(claw_compose_pg_service)"
echo "Postgres ready (${pg}, port ${CLAW_GATEWAY_PG_HOST_PORT:-5433} on host)."

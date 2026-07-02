#!/usr/bin/env bash
# Stop standalone infra PostgreSQL (data volume kept). Author: kejiqing
set -euo pipefail

LIB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PODMAN_DIR="$(cd "${LIB_DIR}/.." && pwd)"
REPO_ROOT="$(cd "${PODMAN_DIR}/../.." && pwd)"
ENV_FILE="${CLAW_INFRA_PG_ENV_FILE:-${REPO_ROOT}/.env}"

# shellcheck disable=SC1091
source "${LIB_DIR}/compose-include.sh"

RT="$(claw_container_runtime_cli)" || exit 1
PROJECT="${CLAW_INFRA_PG_COMPOSE_PROJECT:-claw-infra-pg}"

if [[ -f "${ENV_FILE}" ]]; then
  set -a
  # shellcheck disable=SC1090
  source "${ENV_FILE}"
  set +a
fi

"${RT}" compose -p "${PROJECT}" -f "${PODMAN_DIR}/podman-compose.postgres-only.yml" \
  ${ENV_FILE:+--env-file "${ENV_FILE}"} down
echo "infra postgres stopped (data kept under ${CLAW_PG_DATA_DIR:-${PODMAN_DIR}/claw-pg-data-infra})" >&2

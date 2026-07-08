#!/usr/bin/env bash
# Ephemeral Postgres for http-gateway-rs PG integration tests (not gateway dev DB). Author: kejiqing
set -euo pipefail

LIB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PODMAN_DIR="$(cd "${LIB_DIR}/.." && pwd)"

# shellcheck disable=SC1091
source "${LIB_DIR}/compose-include.sh"

RT="$(claw_container_runtime_cli)" || exit 1
PROJECT="${CLAW_GATEWAY_TEST_PG_COMPOSE_PROJECT:-claw-gateway-pg-test}"
PG_CTN="${CLAW_GATEWAY_TEST_PG_CONTAINER:-claw-gateway-postgres-test}"
PG_PORT="${CLAW_GATEWAY_TEST_PG_HOST_PORT:-5434}"
PG_USER="${CLAW_GATEWAY_PG_USER:-claw_gateway}"
PG_PASSWORD="${CLAW_GATEWAY_PG_PASSWORD:-clawGw9Dev_Pg}"
PG_DATABASE="${CLAW_GATEWAY_PG_DATABASE:-claw_gateway}"
PG_DATA="${CLAW_GATEWAY_TEST_PG_DATA_DIR:-${PODMAN_DIR}/claw-postgres-data-test}"

export CLAW_PG_DATA_DIR="${PG_DATA}"
export CLAW_COMPOSE_PG_CONTAINER="${PG_CTN}"
export CLAW_GATEWAY_PG_HOST_PORT="${PG_PORT}"

echo "==> test PG on 127.0.0.1:${PG_PORT} (container ${PG_CTN}, data ${PG_DATA})" >&2

"${RT}" compose -p "${PROJECT}" -f "${PODMAN_DIR}/podman-compose.postgres-only.yml" up -d postgres

for i in $(seq 1 30); do
  if "${RT}" exec "${PG_CTN}" pg_isready -U "${PG_USER}" -d "${PG_DATABASE}" >/dev/null 2>&1; then
    TEST_URL="postgres://${PG_USER}:${PG_PASSWORD}@127.0.0.1:${PG_PORT}/${PG_DATABASE}"
    echo "OK — test postgres ready (${PG_CTN}:${PG_PORT})" >&2
    echo "export CLAW_GATEWAY_TEST_DATABASE_URL='${TEST_URL}'" >&2
    printf '%s\n' "${TEST_URL}"
    exit 0
  fi
  [[ "${i}" -eq 30 ]] && { echo "error: test postgres not healthy after 60s" >&2; exit 1; }
  sleep 2
done

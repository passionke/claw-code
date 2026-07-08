#!/usr/bin/env bash
# Stop test Postgres (data volume kept). Author: kejiqing
set -euo pipefail

LIB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PODMAN_DIR="$(cd "${LIB_DIR}/.." && pwd)"

# shellcheck disable=SC1091
source "${LIB_DIR}/compose-include.sh"

RT="$(claw_container_runtime_cli)" || exit 1
PROJECT="${CLAW_GATEWAY_TEST_PG_COMPOSE_PROJECT:-claw-gateway-pg-test}"
PG_CTN="${CLAW_GATEWAY_TEST_PG_CONTAINER:-claw-gateway-postgres-test}"

"${RT}" compose -p "${PROJECT}" -f "${PODMAN_DIR}/podman-compose.postgres-only.yml" down
echo "Test postgres stopped (${PG_CTN}); volume data retained."

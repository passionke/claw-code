#!/usr/bin/env bash
# Start Claw Web PostgreSQL (conversation store). Author: kejiqing
set -euo pipefail

LIB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PODMAN_DIR="$(cd "${LIB_DIR}/.." && pwd)"
REPO_ROOT="$(cd "${PODMAN_DIR}/../.." && pwd)"
ENV_FILE="${REPO_ROOT}/.env"

set -a
# shellcheck disable=SC1090
[[ -f "${ENV_FILE}" ]] && source "${ENV_FILE}"
set +a

RT="${CLAW_CONTAINER_RUNTIME:-podman}"
if ! command -v "${RT}" >/dev/null 2>&1; then
  RT=podman
fi

cd "${PODMAN_DIR}"
"${RT}" compose -f podman-compose.yml up -d claw-pg

PG_PORT="${CLAW_WEB_PG_PORT:-5433}"
PG_DB="${CLAW_WEB_PG_DB:-claw_web}"
echo "claw-pg: postgresql://127.0.0.1:${PG_PORT}/${PG_DB}" >&2

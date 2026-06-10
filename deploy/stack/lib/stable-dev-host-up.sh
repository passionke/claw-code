#!/usr/bin/env bash
# Dev-stable backend on a Linux host: PG + pool + tap only (no gateway). Independent of CI. Author: kejiqing
set -euo pipefail

LIB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PODMAN_DIR="$(cd "${LIB_DIR}/.." && pwd)"
REPO_ROOT="$(cd "${PODMAN_DIR}/../.." && pwd)"
ENV_FILE="${CLAW_STABLE_DEV_ENV_FILE:-${REPO_ROOT}/.env.dev-stable}"

if [[ ! -f "${ENV_FILE}" ]]; then
  echo "error: missing ${ENV_FILE}" >&2
  echo "hint: cp deploy/stack/env.stable-dev-host.example ${ENV_FILE} && edit LLM keys" >&2
  exit 1
fi

set -a
# shellcheck disable=SC1090
source "${ENV_FILE}"
set +a

# shellcheck disable=SC1091
source "${LIB_DIR}/compose-include.sh"
# shellcheck disable=SC1091
source "${LIB_DIR}/pool-health.sh"
# shellcheck disable=SC1091
source "${LIB_DIR}/claude-tap-local.sh"

RT="$(claw_container_runtime_cli)" || exit 1
PROJECT="${COMPOSE_PROJECT_NAME:-claw-dev-stable}"
PG_CTN="${CLAW_COMPOSE_PG_CONTAINER:-claw-dev-stable-postgres}"
PG_PORT="${CLAW_GATEWAY_PG_HOST_PORT:-5434}"
POOL_PORT="${CLAW_POOL_HTTP_PORT:-9954}"

echo "==> dev-stable backend: PG :${PG_PORT} pool :${POOL_PORT} tap (see CLAUDE_TAP_PUBLISH_PROXY)" >&2
echo "    cluster=${CLAW_CLUSTER_ID:-?} pool_id=${CLAW_POOL_ID:-?}" >&2

# --- PostgreSQL ---
if ! "${RT}" inspect -f '{{.State.Running}}' "${PG_CTN}" 2>/dev/null | grep -q true; then
  echo "==> starting postgres (${PG_CTN})" >&2
  CLAW_PODMAN_COMPOSE_ARGS=( -p "${PROJECT}" -f "${PODMAN_DIR}/podman-compose.postgres-only.yml" )
  export CLAW_PODMAN_COMPOSE_ARGS
  "${RT}" compose -p "${PROJECT}" -f "${PODMAN_DIR}/podman-compose.postgres-only.yml" \
    --env-file "${ENV_FILE}" up -d postgres
else
  echo "==> postgres already running (${PG_CTN})" >&2
fi

for i in $(seq 1 30); do
  if "${RT}" exec "${PG_CTN}" pg_isready -U "${CLAW_GATEWAY_PG_USER:-claw_gateway}" -d "${CLAW_GATEWAY_PG_DATABASE:-claw_gateway}" >/dev/null 2>&1; then
    echo "postgres ready (${PG_CTN}:${PG_PORT})" >&2
    break
  fi
  [[ "${i}" -eq 30 ]] && { echo "error: postgres not healthy" >&2; exit 1; }
  sleep 2
done

# --- Pool ---
export CLAW_POOL_UP_ENV_FILE="${ENV_FILE}"
export CLAW_POOL_DAEMON_DATABASE_URL="${CLAW_POOL_DAEMON_DATABASE_URL:-postgres://claw_gateway:clawGw9Dev_Pg@127.0.0.1:${PG_PORT}/claw_gateway}"
# shellcheck disable=SC1091
source "${LIB_DIR}/claw-pool-registry-env.sh"
claw_export_pool_registry_env "$(claw_pool_rpc_root "${PODMAN_DIR}")"
# shellcheck disable=SC1091
source "${LIB_DIR}/pool-daemon-binary.sh"
BIN="$(claw_ensure_pool_daemon_binary "${PODMAN_DIR}" "${REPO_ROOT}" | tail -n1)"
export CLAW_POOL_DAEMON_BIN="${BIN}"
"${LIB_DIR}/pool-daemon-up.sh" --ensure
claw_assert_host_pool_http_ready "$(claw_pool_rpc_root "${PODMAN_DIR}")" || {
  echo "error: pool not listening on :${POOL_PORT}" >&2
  exit 1
}

# --- clawTap ---
if claw_stack_manages_local_claude_tap; then
  export CLAW_GATEWAY_DATABASE_URL="${CLAW_TAP_DATABASE_URL:-${CLAW_GATEWAY_DATABASE_URL}}"
  claw_claude_tap_start "${PODMAN_DIR}" "${REPO_ROOT}"
  echo "note: Admin clawTap register skipped (no gateway on this host); LLM via Mac gateway.sh up → dev-stable PG" >&2
else
  echo "==> tap: skipped (CLAW_LLM_PROXY=remote or CLAUDE_TAP_MODE=off)" >&2
fi

echo "==> dev-stable backend up" >&2
echo "    PG:      10.22.28.94:${PG_PORT}" >&2
echo "    pool:    http://10.22.28.94:${POOL_PORT}/healthz/live-report" >&2
echo "    tap:     see CLAUDE_TAP_PUBLISH_PROXY in ${ENV_FILE}" >&2

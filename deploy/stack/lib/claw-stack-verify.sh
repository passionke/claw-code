#!/usr/bin/env bash
# Post-deploy truth checks: schema, pool registry, binary capabilities. Fails loud. Author: kejiqing
set -euo pipefail

LIB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PODMAN_DIR="$(cd "${LIB_DIR}/.." && pwd)"
REPO_ROOT="$(cd "${PODMAN_DIR}/../.." && pwd)"
ENV_FILE="${REPO_ROOT}/.env"
# shellcheck source=stack-instance.sh
source "${LIB_DIR}/stack-instance.sh"
RPC_DIR="$(claw_pool_rpc_root "${PODMAN_DIR}")"
POOL_RPC_DIR="$(claw_pool_rpc_root "${PODMAN_DIR}")"
STAMP_FILE="${PODMAN_DIR}/.claw-build-stamp.env"

fail() {
  echo "VERIFY FAIL: $*" >&2
  exit 1
}

ok() {
  echo "VERIFY OK: $*"
}

if [[ ! -f "${ENV_FILE}" ]]; then
  fail "missing ${ENV_FILE}"
fi
set -a
# shellcheck disable=SC1090
source "${ENV_FILE}"
set +a

# shellcheck source=compose-include.sh
source "${PODMAN_DIR}/lib/compose-include.sh"
# shellcheck source=claw-pool-registry-env.sh
source "${LIB_DIR}/claw-pool-registry-env.sh"

RT="$(claw_container_runtime_cli 2>/dev/null || true)"
[[ -n "${RT}" ]] || fail "need docker or podman in PATH for verify"

PG_USER="${CLAW_GATEWAY_PG_USER:-claw_gateway}"
PG_DB="${CLAW_GATEWAY_PG_DATABASE:-claw_gateway}"
PG_CTN="${CLAW_GATEWAY_PG_CONTAINER:-claw-gateway-postgres}"
PG_PORT="${CLAW_GATEWAY_PG_HOST_PORT:-5433}"

psql_q() {
  if claw_compose_uses_local_postgres; then
    "${RT}" exec "${PG_CTN}" psql -U "${PG_USER}" -d "${PG_DB}" -t -A -c "$1"
    return 0
  fi
  local url pg_img
  url="$(claw_pool_daemon_database_url)" || fail "CLAW_GATEWAY_DATABASE_URL unset"
  pg_img="${CLAW_GATEWAY_PG_IMAGE:-docker.io/library/postgres:17-alpine}"
  "${RT}" run --rm "${pg_img}" psql "${url}" -t -A -c "$1"
}

echo "==> [1/6] PostgreSQL schema (gateway + pool registry)"
if claw_compose_uses_local_postgres; then
  if ! _claw_runtime_container_exists "${RT}" "${PG_CTN}"; then
    fail "postgres container ${PG_CTN} missing (gateway.sh pg-up)"
  fi
  if ! "${RT}" inspect -f '{{.State.Running}}' "${PG_CTN}" 2>/dev/null | grep -qx true; then
    fail "postgres container ${PG_CTN} not running (gateway.sh pg-up)"
  fi
else
  ok "external postgres ($(claw_redact_database_url "${CLAW_GATEWAY_DATABASE_URL}"))"
fi

has_claw_pool="$(psql_q "SELECT to_regclass('public.claw_pool') IS NOT NULL;")"
[[ "${has_claw_pool}" == "t" ]] || fail "table claw_pool missing — gateway image too old or migrate did not run; run pack-deploy"

has_pool_id="$(psql_q "SELECT EXISTS (
  SELECT 1 FROM information_schema.columns
  WHERE table_name='gateway_turns' AND column_name='pool_id');")"
[[ "${has_pool_id}" == "t" ]] || fail "gateway_turns.pool_id missing — rebuild gateway-rs and recreate container"

has_worker_name="$(psql_q "SELECT EXISTS (
  SELECT 1 FROM information_schema.columns
  WHERE table_name='gateway_turns' AND column_name='worker_name');")"
[[ "${has_worker_name}" == "t" ]] || fail "gateway_turns.worker_name missing"

has_artifact_content="$(psql_q "SELECT EXISTS (
  SELECT 1 FROM information_schema.columns
  WHERE table_name='gateway_session_artifacts' AND column_name='content');")"
[[ "${has_artifact_content}" == "t" ]] || fail "gateway_session_artifacts.content missing — pool v1 materialize/readback needs migrate() 004 columns"

has_artifact_upsert_key="$(psql_q "SELECT to_regclass('public.gateway_session_artifacts_session_ds_turn_path_key') IS NOT NULL;")"
[[ "${has_artifact_upsert_key}" == "t" ]] || fail "gateway_session_artifacts unique (session_id,ds_id,turn_id,relative_path) missing — upsert_workspace_tar_b64 ON CONFLICT will fail (legacy ds_id PK; proj_id mirrored in 005_proj_id)"

ok "claw_pool + gateway_turns.pool_id/worker_name + session_artifacts pool-v1 schema present"

echo "==> [2/6] Host pool daemon"
if ! claw_interactive_backend_is_e2b; then
  fail "CLAW_INTERACTIVE_BACKEND must be e2b (local claw-sandbox pool removed)"
fi
ok "FC interactive backend — no host claw-pool-daemon"

if [[ -f "${RPC_DIR}/gateway.env" ]]; then
  set -a
  # shellcheck disable=SC1090
  source "${RPC_DIR}/gateway.env"
  set +a
fi
_base_pool_id="$(claw_default_pool_id)"
POOL_ID="${CLAW_POOL_ID:-${_base_pool_id}}"
POOL_HTTP_PORT="${CLAW_POOL_HTTP_PORT:-9944}"

echo "==> [3/6] skip pool HTTP reachability (e2b backend)"
echo "==> [4/6] skip claw_pool registry (e2b backend)"
echo "==> [5/6] skip claw_pool row (e2b backend)"
echo "==> [6/6] skip gateway.env sandbox URL (e2b backend)"

if [[ -f "${STAMP_FILE}" ]]; then
  echo "--- build stamp ---"
  cat "${STAMP_FILE}"
fi

ok "single-pool verify complete"

echo "==> claw-stack-verify: all checks passed"

#!/usr/bin/env bash
# Post-deploy truth checks: schema, pool registry, binary capabilities. Fails loud. Author: kejiqing
set -euo pipefail

LIB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PODMAN_DIR="$(cd "${LIB_DIR}/.." && pwd)"
REPO_ROOT="$(cd "${PODMAN_DIR}/../.." && pwd)"
ENV_FILE="${REPO_ROOT}/.env"
RPC_DIR="${PODMAN_DIR}/.claw-pool-rpc"
STRICT_RPC_DIR="${RPC_DIR}/strict"
RELAXED_RPC_DIR="${RPC_DIR}/relaxed"
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
# shellcheck source=pool-health.sh
source "${LIB_DIR}/pool-health.sh"

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
  "${RT}" ps --format '{{.Names}}' | grep -qx "${PG_CTN}" || fail "postgres container ${PG_CTN} not running (gateway.sh pg-up)"
else
  ok "external postgres (${CLAW_GATEWAY_DATABASE_URL%%@*}@…)"
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

echo "==> [2/6] Host pool daemon (v1: no compose sidecar)"
claw_pool_daemon_on_host || fail "host pool required (compose sidecar removed)"
BIN="${CLAW_POOL_DAEMON_BIN:-$(claw_default_pool_daemon_bin "${PODMAN_DIR}")}"
# shellcheck source=pool-daemon-binary.sh
source "${LIB_DIR}/pool-daemon-binary.sh"
[[ -x "${BIN}" ]] || fail "host claw-pool-daemon not executable: ${BIN}"

if ! python3 -c "import pathlib,sys; b=pathlib.Path(sys.argv[1]).read_bytes(); sys.exit(0 if b'claw_pool registered' in b else 1)" "${BIN}" 2>/dev/null; then
  fail "host ${BIN} lacks 'claw_pool registered' — run pack-deploy or cargo build -p http-gateway-rs --bin claw-pool-daemon"
fi
if ! python3 -c "import pathlib,sys; b=pathlib.Path(sys.argv[1]).read_bytes(); sys.exit(0 if b'assign_turn_pool_worker_ok' in b else 1)" "${BIN}" 2>/dev/null; then
  fail "host ${BIN} lacks assign_turn_pool_worker_ok — stale binary"
fi
ok "pool daemon binary contains registry + turn-assignment strings"

claw_verify_pool_profile() {
  local profile="$1" rpc_dir="$2" pool_id="$3" http_port="$4"
  export CLAW_POOL_HTTP_PORT="${http_port}"
  export CLAW_POOL_ID="${pool_id}"
  echo "==> pool verify [${profile}] pool_id=${pool_id} HTTP :${http_port}"
  [[ -d "${rpc_dir}" ]] || fail "missing ${rpc_dir} — run gateway.sh pool-up"
  claw_assert_host_pool_http_ready "${rpc_dir}" || fail "host ${profile} pool HTTP not ready on 127.0.0.1:${http_port}"
  [[ -f "${rpc_dir}/pool-registry.env" ]] || fail "missing ${rpc_dir}/pool-registry.env"
  LOG="${rpc_dir}/daemon.log"
  [[ -f "${LOG}" ]] || fail "missing ${LOG}"
  if tail -200 "${LOG}" | grep -q "claw_pool registry disabled"; then
    tail -30 "${LOG}" >&2
    fail "${profile} pool registry disabled in daemon.log"
  fi
  if ! tail -200 "${LOG}" | grep -q "claw_pool registered"; then
    tail -30 "${LOG}" >&2
    fail "no claw_pool registered in ${profile} daemon.log"
  fi
  ok "${profile} daemon.log shows claw_pool registered"
  row_pool_id="$(psql_q "SELECT pool_id FROM claw_pool WHERE pool_id='${pool_id}' LIMIT 1;")"
  [[ "${row_pool_id}" == "${pool_id}" ]] || fail "claw_pool has no row for ${profile} pool_id=${pool_id}"
  hb="$(psql_q "SELECT (EXTRACT(EPOCH FROM NOW())*1000 - last_heartbeat_ms) < 120000 FROM claw_pool WHERE pool_id='${pool_id}';")"
  [[ "${hb}" == "t" ]] || fail "claw_pool heartbeat stale (>120s) for ${pool_id}"
  ok "claw_pool row ${pool_id} heartbeat fresh"
  claw_assert_host_pool_rpc_ready "${rpc_dir}" || fail "${profile} pool RPC died during verify"
}

if [[ -f "${RPC_DIR}/gateway.env" ]]; then
  set -a
  # shellcheck disable=SC1090
  source "${RPC_DIR}/gateway.env"
  set +a
fi
_base_pool_id="$(claw_default_pool_id)"
STRICT_POOL_ID="${CLAW_STRICT_POOL_ID:-${_base_pool_id}-strict}"
RELAXED_POOL_ID="${CLAW_RELAXED_POOL_ID:-${_base_pool_id}-relaxed}"
STRICT_HTTP_PORT="${CLAW_STRICT_POOL_HTTP_PORT:-9944}"
RELAXED_HTTP_PORT="${CLAW_RELAXED_POOL_HTTP_PORT:-9954}"

claw_assert_gateway_pool_http_reachable "${PODMAN_DIR}" \
  || fail "gateway container cannot reach strict pool HTTP — run gateway.sh up"

echo "==> [3/6] dual pool registry (strict + relaxed)"
if [[ -f "${LIB_DIR}/pool-daemon-systemd.sh" ]]; then
  # shellcheck disable=SC1091
  source "${LIB_DIR}/pool-daemon-systemd.sh"
  claw_pool_systemd_assert_dual_pool_coherent "${PODMAN_DIR}" \
    || fail "systemd dual-pool incoherent (legacy single unit may have overwritten strict)"
fi
claw_verify_pool_profile strict "${STRICT_RPC_DIR}" "${STRICT_POOL_ID}" "${STRICT_HTTP_PORT}"
claw_verify_pool_profile relaxed "${RELAXED_RPC_DIR}" "${RELAXED_POOL_ID}" "${RELAXED_HTTP_PORT}"

echo "==> [4/6] pool daemon DB URL (host must not use compose hostname postgres)"
pool_db_url="$(claw_pool_daemon_database_url)" || fail "CLAW_GATEWAY_DATABASE_URL unset"
case "${pool_db_url}" in
  *@postgres:*)
    fail "host pool would use @postgres: — use 127.0.0.1:${PG_PORT} (claw_pool_daemon_database_url)"
    ;;
esac
ok "host pool DB URL uses reachable host (${pool_db_url%%@*}@…)"

echo "==> [5/6] claw_pool table has both profiles"
pool_rows="$(psql_q "SELECT count(*)::text FROM claw_pool WHERE pool_id IN ('${STRICT_POOL_ID}','${RELAXED_POOL_ID}');")"
[[ "${pool_rows}" -ge 2 ]] || fail "claw_pool expected 2 rows (strict+relaxed), got ${pool_rows}"
ok "claw_pool has strict + relaxed rows"

echo "==> [6/6] gateway.env dual pool RPC bases"
[[ -f "${RPC_DIR}/gateway.env" ]] || fail "missing gateway.env"
grep -q '^CLAW_STRICT_POOL_HTTP_BASE=' "${RPC_DIR}/gateway.env" \
  || fail "gateway.env missing CLAW_STRICT_POOL_HTTP_BASE"
grep -q '^CLAW_RELAXED_POOL_HTTP_BASE=' "${RPC_DIR}/gateway.env" \
  || fail "gateway.env missing CLAW_RELAXED_POOL_HTTP_BASE"
ok "gateway.env lists strict + relaxed pool HTTP bases"

if [[ -f "${STAMP_FILE}" ]]; then
  echo "--- build stamp ---"
  cat "${STAMP_FILE}"
fi

ok "dual pool verify complete"

echo "==> claw-stack-verify: all checks passed"

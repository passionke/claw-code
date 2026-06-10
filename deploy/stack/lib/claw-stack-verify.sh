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
STRICT_RPC_DIR="$(claw_strict_pool_rpc_dir "${PODMAN_DIR}")"
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
[[ -x "${BIN}" ]] || fail "host claw-sandbox not executable: ${BIN}"

if ! python3 -c "import pathlib,sys; b=pathlib.Path(sys.argv[1]).read_bytes(); sys.exit(0 if b'claw_pool registered' in b else 1)" "${BIN}" 2>/dev/null; then
  fail "host ${BIN} lacks 'claw_pool registered' — run gateway.sh build or cargo build -p claw-sandbox-server in sandbox/"
fi
if ! python3 -c "import pathlib,sys; b=pathlib.Path(sys.argv[1]).read_bytes(); sys.exit(0 if (b'assign_turn_pool_worker_ok' in b or b'acquire_slot_ok' in b or b'claw-sandbox:' in b) else 1)" "${BIN}" 2>/dev/null; then
  fail "host ${BIN} looks stale (expected claw-sandbox or legacy claw-pool-daemon markers)"
fi
ok "pool daemon binary contains registry + pool markers"

# Lines after the last pool-daemon-up start (ignore stale errors from prior runs). kejiqing
claw_pool_daemon_log_current_run() {
  local log="${1:?log}"
  [[ -f "${log}" ]] || return 1
  awk '/pool-daemon-up: starting/{buf=""; on=1; next} on{buf=buf $0 ORS} END{printf "%s", buf}' "${log}"
}

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
  run_log="$(claw_pool_daemon_log_current_run "${LOG}")"
  if [[ -z "${run_log}" ]]; then
    tail -30 "${LOG}" >&2
    fail "${profile} daemon.log has no lines after last pool-daemon-up start"
  fi
  if printf '%s' "${run_log}" | grep -q "claw_pool registry disabled"; then
    printf '%s\n' "${run_log}" | tail -30 >&2
    fail "${profile} pool registry disabled in current daemon run"
  fi
  if ! printf '%s' "${run_log}" | grep -q "claw_pool registered"; then
    printf '%s' "${run_log}" | tail -30 >&2
    fail "no claw_pool registered in current ${profile} daemon run"
  fi
  ok "${profile} daemon.log shows claw_pool registered"
  row_pool_id="$(psql_q "SELECT pool_id FROM claw_pool WHERE pool_id='${pool_id}' LIMIT 1;")"
  [[ "${row_pool_id}" == "${pool_id}" ]] || fail "claw_pool has no row for ${profile} pool_id=${pool_id}"
  hb="$(psql_q "SELECT (EXTRACT(EPOCH FROM NOW())*1000 - last_heartbeat_ms) < 120000 FROM claw_pool WHERE pool_id='${pool_id}';")"
  [[ "${hb}" == "t" ]] || fail "claw_pool heartbeat stale (>120s) for ${pool_id}"
  ok "claw_pool row ${pool_id} heartbeat fresh"
  claw_assert_host_pool_rpc_ready "${rpc_dir}" || fail "${profile} pool RPC died during verify"
}

# POST /v1/sandbox/rpc Capacity — unified pool must expose strict (+ relaxed when enabled). kejiqing
claw_verify_sandbox_capacity_profiles() {
  local http_port="$1"
  local want_relaxed="$2"
  local body resp
  body='{"op":"capacity"}'
  resp="$(curl -fsS --connect-timeout 5 -X POST \
    "http://127.0.0.1:${http_port}/v1/sandbox/rpc" \
    -H 'Content-Type: application/json' \
    -d "${body}" 2>/dev/null)" \
    || fail "sandbox Capacity RPC failed on :${http_port}"
  if ! python3 -c '
import json, sys
want_relaxed = sys.argv[1] == "1"
d = json.loads(sys.argv[2])
if not d.get("ok"):
    raise SystemExit("capacity not ok: " + str(d.get("error")))
cap = d.get("capacity") or {}
profiles = {p.get("profile") for p in (cap.get("profiles") or [])}
if "strict" not in profiles:
    raise SystemExit("missing strict profile in capacity.profiles: " + str(profiles))
if want_relaxed and "relaxed" not in profiles:
    raise SystemExit("missing relaxed profile in capacity.profiles: " + str(profiles))
' "${want_relaxed}" "${resp}"; then
    fail "sandbox capacity profiles check failed (port ${http_port})"
  fi
  ok "sandbox capacity lists required worker profiles"
}

if [[ -f "${RPC_DIR}/gateway.env" ]]; then
  set -a
  # shellcheck disable=SC1090
  source "${RPC_DIR}/gateway.env"
  set +a
fi
_base_pool_id="$(claw_default_pool_id)"
POOL_ID="${CLAW_POOL_ID:-${CLAW_STRICT_POOL_ID:-${_base_pool_id}}}"
POOL_HTTP_PORT="${CLAW_STRICT_POOL_HTTP_PORT:-9944}"

claw_assert_gateway_pool_http_reachable "${PODMAN_DIR}" \
  || fail "gateway container cannot reach pool HTTP — run gateway.sh up"

echo "==> [3/6] single claw-sandbox registry"
claw_verify_pool_profile sandbox "${STRICT_RPC_DIR}" "${POOL_ID}" "${POOL_HTTP_PORT}"
if claw_relaxed_worker_allowed_from_env; then
  claw_verify_sandbox_capacity_profiles "${POOL_HTTP_PORT}" 1
else
  claw_verify_sandbox_capacity_profiles "${POOL_HTTP_PORT}" 0
fi

echo "==> [4/6] pool daemon DB URL (host must not use compose hostname postgres)"
pool_db_url="$(claw_pool_daemon_database_url)" || fail "CLAW_GATEWAY_DATABASE_URL unset"
case "${pool_db_url}" in
  *@postgres:*)
    fail "host pool would use @postgres: — use 127.0.0.1:${PG_PORT} (claw_pool_daemon_database_url)"
    ;;
esac
ok "host pool DB URL uses reachable host (${pool_db_url%%@*}@…)"

echo "==> [5/6] claw_pool registry row"
row_pool="$(psql_q "SELECT pool_id FROM claw_pool WHERE pool_id='${POOL_ID}' LIMIT 1;")"
[[ "${row_pool}" == "${POOL_ID}" ]] || fail "claw_pool missing row pool_id=${POOL_ID}"
ok "claw_pool has row ${POOL_ID}"

echo "==> [6/6] gateway.env sandbox URL"
[[ -f "${RPC_DIR}/gateway.env" ]] || fail "missing gateway.env"
grep -q '^CLAW_SANDBOX_URL=' "${RPC_DIR}/gateway.env" \
  || fail "gateway.env missing CLAW_SANDBOX_URL"
grep -q '^CLAW_POOL_HTTP_BASE=' "${RPC_DIR}/gateway.env" \
  || fail "gateway.env missing CLAW_POOL_HTTP_BASE"
ok "gateway.env lists CLAW_SANDBOX_URL + CLAW_POOL_HTTP_BASE"

if [[ -f "${STAMP_FILE}" ]]; then
  echo "--- build stamp ---"
  cat "${STAMP_FILE}"
fi

ok "single-pool verify complete"

echo "==> claw-stack-verify: all checks passed"

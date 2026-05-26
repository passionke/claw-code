#!/usr/bin/env bash
# Post-deploy truth checks: schema, pool registry, binary capabilities. Fails loud. Author: kejiqing
set -euo pipefail

LIB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PODMAN_DIR="$(cd "${LIB_DIR}/.." && pwd)"
REPO_ROOT="$(cd "${PODMAN_DIR}/../.." && pwd)"
ENV_FILE="${REPO_ROOT}/.env"
RPC_DIR="${PODMAN_DIR}/.claw-pool-rpc"
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

PG_USER="${CLAW_GATEWAY_PG_USER:-claw_gateway}"
PG_DB="${CLAW_GATEWAY_PG_DATABASE:-claw_gateway}"
PG_CTN="${CLAW_GATEWAY_PG_CONTAINER:-claw-gateway-postgres}"
PG_PORT="${CLAW_GATEWAY_PG_HOST_PORT:-5433}"

psql_q() {
  podman exec "${PG_CTN}" psql -U "${PG_USER}" -d "${PG_DB}" -t -A -c "$1"
}

echo "==> [1/6] PostgreSQL schema (gateway + pool registry)"
podman ps --format '{{.Names}}' | grep -qx "${PG_CTN}" || fail "postgres container ${PG_CTN} not running (gateway.sh pg-up)"

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
ok "claw_pool + gateway_turns.pool_id/worker_name present"

echo "==> [2/6] Host pool daemon process + binary capabilities"
if ! claw_pool_daemon_on_host; then
  ok "pool runs in compose sidecar (skip host daemon checks; run verify inside Linux deploy host)"
  exit 0
fi

BIN="${CLAW_POOL_DAEMON_BIN:-${REPO_ROOT}/rust/target/release/claw-pool-daemon}"
[[ -x "${BIN}" ]] || fail "host claw-pool-daemon not executable: ${BIN}"

if ! python3 -c "import pathlib,sys; b=pathlib.Path(sys.argv[1]).read_bytes(); sys.exit(0 if b'claw_pool registered' in b else 1)" "${BIN}" 2>/dev/null; then
  fail "host ${BIN} lacks 'claw_pool registered' — stale binary; run: cargo build --release -p http-gateway-rs --bin claw-pool-daemon"
fi
if ! python3 -c "import pathlib,sys; b=pathlib.Path(sys.argv[1]).read_bytes(); sys.exit(0 if b'assign_turn_pool_worker_ok' in b else 1)" "${BIN}" 2>/dev/null; then
  fail "host ${BIN} lacks assign_turn_pool_worker_ok — stale binary"
fi
ok "pool daemon binary contains registry + turn-assignment strings"

[[ -f "${RPC_DIR}/daemon.pid" ]] || fail "missing ${RPC_DIR}/daemon.pid — pool-daemon-up did not run"
dpid="$(cat "${RPC_DIR}/daemon.pid")"
kill -0 "${dpid}" 2>/dev/null || fail "claw-pool-daemon pid ${dpid} not running"

echo "==> [3/6] pool-registry.env"
[[ -f "${RPC_DIR}/pool-registry.env" ]] || fail "missing pool-registry.env — up.sh must run claw_export_pool_registry_env"
# shellcheck disable=SC1090
source "${RPC_DIR}/pool-registry.env"
[[ -n "${CLAW_POOL_ID:-}" ]] || fail "CLAW_POOL_ID empty in pool-registry.env"
[[ -n "${CLAW_POOL_ADVERTISE_HOST:-}" ]] || fail "CLAW_POOL_ADVERTISE_HOST empty"
ok "pool-registry.env pool_id=${CLAW_POOL_ID} advertise=${CLAW_POOL_ADVERTISE_HOST}"

echo "==> [4/6] pool daemon DB URL (host must not use compose hostname postgres)"
pool_db_url="$(claw_pool_daemon_database_url)" || fail "CLAW_GATEWAY_DATABASE_URL unset"
case "${pool_db_url}" in
  *@postgres:*)
    fail "host pool would use @postgres: — use 127.0.0.1:${PG_PORT} (claw_pool_daemon_database_url)"
    ;;
esac
ok "host pool DB URL uses reachable host (${pool_db_url%%@*}@…)"

echo "==> [5/6] daemon.log — registry succeeded (not disabled)"
LOG="${RPC_DIR}/daemon.log"
[[ -f "${LOG}" ]] || fail "missing ${LOG}"
if tail -200 "${LOG}" | grep -q "claw_pool registry disabled"; then
  tail -30 "${LOG}" >&2
  fail "pool registry disabled in daemon.log (often postgres hostname from host)"
fi
if ! tail -200 "${LOG}" | grep -q "claw_pool registered"; then
  tail -30 "${LOG}" >&2
  fail "no 'claw_pool registered' in recent daemon.log"
fi
ok "daemon.log shows claw_pool registered"

echo "==> [6/6] claw_pool row + heartbeat"
pool_rows="$(psql_q "SELECT count(*)::text FROM claw_pool;")"
[[ "${pool_rows}" -ge 1 ]] || fail "claw_pool empty — registry did not persist"
row_pool_id="$(psql_q "SELECT pool_id FROM claw_pool WHERE pool_id='${CLAW_POOL_ID}' LIMIT 1;")"
[[ "${row_pool_id}" == "${CLAW_POOL_ID}" ]] || fail "claw_pool has no row for CLAW_POOL_ID=${CLAW_POOL_ID}"
hb="$(psql_q "SELECT (EXTRACT(EPOCH FROM NOW())*1000 - last_heartbeat_ms) < 120000 FROM claw_pool WHERE pool_id='${CLAW_POOL_ID}';")"
[[ "${hb}" == "t" ]] || fail "claw_pool heartbeat stale (>120s) for ${CLAW_POOL_ID}"
ok "claw_pool row ${CLAW_POOL_ID} heartbeat fresh"

if [[ -f "${STAMP_FILE}" ]]; then
  echo "--- build stamp ---"
  cat "${STAMP_FILE}"
fi

echo "==> claw-stack-verify: all checks passed"

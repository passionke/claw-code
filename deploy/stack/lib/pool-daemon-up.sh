#!/usr/bin/env bash
# Start host `claw-pool-daemon` on TCP. Binary path from gateway.sh up (install under deploy/stack/.linux-artifacts/). Author: kejiqing
set -euo pipefail
LIB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PODMAN_DIR="$(cd "${LIB_DIR}/.." && pwd)"
REPO_ROOT="$(cd "${PODMAN_DIR}/../.." && pwd)"
# shellcheck disable=SC1091
source "${LIB_DIR}/nuclear-pool-reset.sh"
# shellcheck disable=SC1091
source "${LIB_DIR}/compose-include.sh"

# gateway.sh up already sourced .env and set CLAW_POOL_DAEMON_BIN (release install under
# deploy/stack/.linux-artifacts/). Re-sourcing .env here would resurrect a stale
# CLAW_POOL_DAEMON_BIN=~/.local/bin/... and break worker --entrypoint sleep. kejiqing
_pool_bin_from_up="${CLAW_POOL_DAEMON_BIN:-}"
if [[ -f "${REPO_ROOT}/.env" ]]; then
  set -a
  # shellcheck disable=SC1090
  source "${REPO_ROOT}/.env"
  set +a
fi
if [[ -n "${_pool_bin_from_up}" ]]; then
  export CLAW_POOL_DAEMON_BIN="${_pool_bin_from_up}"
fi

claw_ensure_worker_llm_wiring "${PODMAN_DIR}"
# Release/sticky pin overrides CLAW_DOCKER_IMAGE from repo .env (e.g. claw-gateway-worker:local). kejiqing
claw_reapply_pool_image_pins "${PODMAN_DIR}"

if [[ -z "${CLAW_POOL_WORK_ROOT_BIND_SRC:-}" ]]; then
  claw_podman_export_pool_workspace "${PODMAN_DIR}"
fi

WORK_ROOT="${CLAW_POOL_WORK_ROOT_BIND_SRC:?missing CLAW_POOL_WORK_ROOT_BIND_SRC; run gateway.sh up first}"
RPC_DIR="${PODMAN_DIR}/.claw-pool-rpc"
BIN="${CLAW_POOL_DAEMON_BIN:-$(claw_default_pool_daemon_bin "${PODMAN_DIR}")}"
PORT="${CLAW_POOL_DAEMON_PORT:-9943}"
BIND="0.0.0.0:${PORT}"

mkdir -p "${RPC_DIR}"

if [[ -f "${RPC_DIR}/daemon.pid" ]]; then
  old="$(cat "${RPC_DIR}/daemon.pid")"
  if kill -0 "${old}" 2>/dev/null; then
    kill "${old}" 2>/dev/null || true
    sleep 0.2
  fi
  rm -f "${RPC_DIR}/daemon.pid"
fi
claw_kill_tcp_listeners "${PORT}"

claw_remove_all_gateway_workers

# shellcheck source=claw-pool-registry-env.sh
source "${LIB_DIR}/claw-pool-registry-env.sh"
claw_export_pool_registry_env "${RPC_DIR}"

if [[ ! -x "${BIN}" ]]; then
  echo "error: claw-pool-daemon missing or not executable: ${BIN}" >&2
  echo "hint: ./deploy/stack/gateway.sh build then ./deploy/stack/gateway.sh up (installs from GATEWAY_IMAGE), or:" >&2
  echo "  ./deploy/stack/lib/install-pool-daemon-from-image.sh ${BIN}" >&2
  exit 1
fi

daemon_env=(
  CLAW_WORK_ROOT="${WORK_ROOT}"
  CLAW_POOL_WORK_ROOT_HOST="${WORK_ROOT}"
  CLAW_POOL_DAEMON_TCP_BIND="${BIND}"
  CLAW_SOLVE_ISOLATION="${CLAW_SOLVE_ISOLATION:-podman_pool}"
  CLAW_WORKER_ENV_FILE="${CLAW_WORKER_ENV_FILE:-${REPO_ROOT}/.env}"
)
if [[ -n "${CLAW_DOCKER_IMAGE:-}" ]]; then
  daemon_env+=(CLAW_DOCKER_IMAGE="${CLAW_DOCKER_IMAGE}")
fi
if [[ -n "${CLAW_PODMAN_IMAGE:-}" ]]; then
  daemon_env+=(CLAW_PODMAN_IMAGE="${CLAW_PODMAN_IMAGE}")
fi
if [[ -n "${CLAW_PODMAN_NETWORK:-}" ]]; then
  daemon_env+=(CLAW_PODMAN_NETWORK="${CLAW_PODMAN_NETWORK}")
fi
if [[ -n "${CLAW_DOCKER_EXTRA_ARGS:-}" ]]; then
  daemon_env+=(CLAW_DOCKER_EXTRA_ARGS="${CLAW_DOCKER_EXTRA_ARGS}")
fi
if [[ -n "${CLAW_PODMAN_EXTRA_ARGS:-}" ]]; then
  daemon_env+=(CLAW_PODMAN_EXTRA_ARGS="${CLAW_PODMAN_EXTRA_ARGS}")
fi
daemon_env+=(
  CLAW_POOL_HTTP_BIND="0.0.0.0:${CLAW_POOL_HTTP_PORT:-9944}"
  CLAW_POOL_ADVERTISE_HOST="${CLAW_POOL_ADVERTISE_HOST}"
  CLAW_POOL_ID="${CLAW_POOL_ID}"
)
if pool_db_url="$(claw_pool_daemon_database_url)"; then
  daemon_env+=(CLAW_GATEWAY_DATABASE_URL="${pool_db_url}")
fi
nohup env "${daemon_env[@]}" "${BIN}" >>"${RPC_DIR}/daemon.log" 2>&1 &
dpid=$!
echo "${dpid}" >"${RPC_DIR}/daemon.pid"

sleep 0.15
if ! kill -0 "${dpid}" 2>/dev/null; then
  wait "${dpid}" 2>/dev/null || true
  rc=$?
  echo "error: claw-pool-daemon exited immediately (pid ${dpid}, wait status ${rc})" >&2
  if command -v file >/dev/null 2>&1; then
    file "${BIN}" >&2 || true
  fi
  if command -v ldd >/dev/null 2>&1; then
    ldd "${BIN}" >&2 || true
  fi
  echo "hint: reinstall from GATEWAY_IMAGE (must match gateway tag):" >&2
  echo "  ./deploy/stack/lib/install-pool-daemon-from-image.sh ${BIN}" >&2
  echo "  do not copy rust/target/release/claw-pool-daemon from macOS to Linux" >&2
  tail -40 "${RPC_DIR}/daemon.log" >&2 || true
  exit 1
fi

for _ in $(seq 1 100); do
  if python3 -c "import socket; s=socket.socket(); s.settimeout(0.2); s.connect(('127.0.0.1', int('${PORT}'))); s.close()" 2>/dev/null; then
    echo "claw-pool-daemon listening on 127.0.0.1:${PORT} pool_id=${CLAW_POOL_ID} advertise=${CLAW_POOL_ADVERTISE_HOST} worker=${CLAW_DOCKER_IMAGE:-${CLAW_PODMAN_IMAGE:-unset}}" >&2
    if pool_db_url="$(claw_pool_daemon_database_url 2>/dev/null || true)" && [[ -n "${pool_db_url}" ]]; then
      for _w in $(seq 1 80); do
        if grep -q "claw_pool registered" "${RPC_DIR}/daemon.log" 2>/dev/null; then
          echo "claw-pool-daemon: claw_pool registered in PostgreSQL" >&2
          exit 0
        fi
        if tail -5 "${RPC_DIR}/daemon.log" 2>/dev/null | grep -q "claw_pool registry disabled"; then
          echo "error: claw-pool-daemon started but pool registry disabled (check CLAW_GATEWAY_DATABASE_URL / host port ${CLAW_GATEWAY_PG_HOST_PORT:-5433})" >&2
          tail -20 "${RPC_DIR}/daemon.log" >&2 || true
          exit 1
        fi
        sleep 0.1
      done
      echo "error: pool listens on TCP but claw_pool not registered within 8s — stale binary or DB unreachable" >&2
      tail -30 "${RPC_DIR}/daemon.log" >&2 || true
      exit 1
    fi
    exit 0
  fi
  sleep 0.05
done

echo "claw-pool-daemon did not accept TCP 127.0.0.1:${PORT}; tail ${RPC_DIR}/daemon.log:" >&2
tail -40 "${RPC_DIR}/daemon.log" >&2 || true
exit 1

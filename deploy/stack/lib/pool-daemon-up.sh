#!/usr/bin/env bash
# Start host `claw-pool-daemon` on TCP. Binary must already exist (gateway.sh up runs install-pool-daemon-from-image.sh first). Author: kejiqing
set -euo pipefail
LIB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PODMAN_DIR="$(cd "${LIB_DIR}/.." && pwd)"
REPO_ROOT="$2"
# shellcheck disable=SC1091
source "${LIB_DIR}/nuclear-pool-reset.sh"
# Always refresh worker env snapshot from repo-root .env before (re)starting the daemon so pool
# workers never inherit stale keys (e.g. CLAW_MCP_TOOL_CALL_TIMEOUT_MS). Author: kejiqing
"${PODMAN_DIR}/lib/sync-worker-openai-env.sh"
WORK_ROOT="${CLAW_POOL_WORK_ROOT_BIND_SRC:?missing CLAW_POOL_WORK_ROOT_BIND_SRC}"
RPC_DIR="${PODMAN_DIR}/.claw-pool-rpc"
BIN="${CLAW_POOL_DAEMON_BIN:-${REPO_ROOT}/rust/target/release/claw-pool-daemon}"
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

# shellcheck source=/dev/null
source "${PODMAN_DIR}/lib/compose-include.sh"
claw_remove_all_gateway_workers

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
)
if [[ -n "${CLAW_DOCKER_IMAGE:-}" ]]; then
  daemon_env+=(CLAW_DOCKER_IMAGE="${CLAW_DOCKER_IMAGE}")
fi
if [[ -n "${CLAW_PODMAN_IMAGE:-}" ]]; then
  daemon_env+=(CLAW_PODMAN_IMAGE="${CLAW_PODMAN_IMAGE}")
fi
nohup env "${daemon_env[@]}" "${BIN}" >>"${RPC_DIR}/daemon.log" 2>&1 &
echo $! >"${RPC_DIR}/daemon.pid"

for _ in $(seq 1 100); do
  if python3 -c "import socket; s=socket.socket(); s.settimeout(0.2); s.connect(('127.0.0.1', int('${PORT}'))); s.close()" 2>/dev/null; then
    echo "claw-pool-daemon listening on 127.0.0.1:${PORT} worker=${CLAW_DOCKER_IMAGE:-${CLAW_PODMAN_IMAGE:-unset}}" >&2
    exit 0
  fi
  sleep 0.05
done

echo "claw-pool-daemon did not accept TCP 127.0.0.1:${PORT}; tail ${RPC_DIR}/daemon.log:" >&2
tail -40 "${RPC_DIR}/daemon.log" >&2 || true
exit 1

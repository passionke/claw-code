#!/usr/bin/env bash
# Stop host `claw-pool-daemon` and free the RPC port (avoid duplicate daemons). Author: kejiqing
set -euo pipefail
LIB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PODMAN_DIR="$(cd "${LIB_DIR}/.." && pwd)"
RPC_DIR="${PODMAN_DIR}/.claw-pool-rpc"
PORT="${CLAW_POOL_DAEMON_PORT:-9943}"

# shellcheck disable=SC1091
source "${LIB_DIR}/nuclear-pool-reset.sh"

if [[ -f "${RPC_DIR}/daemon.pid" ]]; then
  pid="$(cat "${RPC_DIR}/daemon.pid")"
  if kill -0 "${pid}" 2>/dev/null; then
    kill "${pid}" 2>/dev/null || true
    sleep 0.2
  fi
  rm -f "${RPC_DIR}/daemon.pid"
fi

# Orphans (e.g. manual nohup) and stale listeners on the RPC port.
if command -v pkill >/dev/null 2>&1; then
  pkill -f 'claw-pool-daemon' 2>/dev/null || true
fi
claw_kill_tcp_listeners "${PORT}"
rm -f "${RPC_DIR}/pool.sock"

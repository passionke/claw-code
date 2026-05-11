#!/usr/bin/env bash
# Stop host `claw-pool-daemon`. Author: kejiqing
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
RPC_DIR="${SCRIPT_DIR}/.claw-pool-rpc"
if [[ -f "${RPC_DIR}/daemon.pid" ]]; then
  pid="$(cat "${RPC_DIR}/daemon.pid")"
  if kill -0 "${pid}" 2>/dev/null; then
    kill "${pid}" 2>/dev/null || true
  fi
  rm -f "${RPC_DIR}/daemon.pid"
fi
rm -f "${RPC_DIR}/pool.sock"

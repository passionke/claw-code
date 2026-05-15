#!/usr/bin/env bash
# Start host `claw-pool-daemon` on TCP. Binary must already exist (gateway.sh up runs install-pool-daemon-from-image.sh first). Author: kejiqing
set -euo pipefail
PODMAN_DIR="$1"
REPO_ROOT="$2"
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

# Drop every pool worker so the next daemon warm pass always `docker run`s fresh containers that
# read the just-synced worker-openai.env (stack / daemon restart must not reuse stale workers). kejiqing
# shellcheck source=/dev/null
source "${PODMAN_DIR}/lib/compose-include.sh"
rt="$(claw_container_runtime_cli)" || {
  echo "error: pool-daemon-up needs docker or podman in PATH to remove stale claw-worker-* / claw-gw-* workers" >&2
  exit 1
}
ids_w="$("${rt}" ps -aq --filter name='claw-worker-' 2>/dev/null || true)"
ids_g="$("${rt}" ps -aq --filter name='claw-gw-' 2>/dev/null || true)"
ids="${ids_w} ${ids_g}"
if [[ -n "${ids//[$'\t\r\n ']}" ]]; then
  echo "Removing pool worker containers before pool-daemon (re)start…" >&2
  # shellcheck disable=SC2086
  "${rt}" rm -f ${ids}
fi

if [[ ! -x "${BIN}" ]]; then
  echo "error: claw-pool-daemon missing or not executable: ${BIN}" >&2
  echo "hint: ./deploy/stack/gateway.sh build then ./deploy/stack/gateway.sh up (installs from GATEWAY_IMAGE), or:" >&2
  echo "  ./deploy/stack/lib/install-pool-daemon-from-image.sh ${BIN}" >&2
  exit 1
fi

nohup env \
  CLAW_WORK_ROOT="${WORK_ROOT}" \
  CLAW_POOL_WORK_ROOT_HOST="${WORK_ROOT}" \
  CLAW_POOL_DAEMON_TCP_BIND="${BIND}" \
  CLAW_SOLVE_ISOLATION="${CLAW_SOLVE_ISOLATION:-podman_pool}" \
  "${BIN}" >>"${RPC_DIR}/daemon.log" 2>&1 &
echo $! >"${RPC_DIR}/daemon.pid"

for _ in $(seq 1 100); do
  if python3 -c "import socket; s=socket.socket(); s.settimeout(0.2); s.connect(('127.0.0.1', int('${PORT}'))); s.close()" 2>/dev/null; then
    exit 0
  fi
  sleep 0.05
done

echo "claw-pool-daemon did not accept TCP 127.0.0.1:${PORT}; tail ${RPC_DIR}/daemon.log:" >&2
tail -40 "${RPC_DIR}/daemon.log" >&2 || true
exit 1

#!/usr/bin/env bash
# Start host `claw-pool-daemon` on TCP. Dev: cargo build. Prod: CLAW_POOL_DAEMON_SKIP_BUILD=1 + CLAW_POOL_DAEMON_BIN (e.g. from GHCR image). Author: kejiqing
set -euo pipefail
SCRIPT_DIR="$1"
REPO_ROOT="$2"
WORK_ROOT="${CLAW_POOL_WORK_ROOT_BIND_SRC:?missing CLAW_POOL_WORK_ROOT_BIND_SRC}"
RPC_DIR="${SCRIPT_DIR}/.claw-pool-rpc"
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

if [[ "${CLAW_POOL_DAEMON_SKIP_BUILD:-0}" == "1" ]]; then
  if [[ ! -x "${BIN}" ]]; then
    echo "error: CLAW_POOL_DAEMON_SKIP_BUILD=1 but missing or not executable: ${BIN}" >&2
    echo "hint: ./deploy/podman/install-pool-daemon-from-image.sh /usr/local/bin/claw-pool-daemon" >&2
    exit 1
  fi
else
  echo "cargo build claw-pool-daemon (release) …" >&2
  (cd "${REPO_ROOT}/rust" && cargo build -p http-gateway-rs --bin claw-pool-daemon --release)
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

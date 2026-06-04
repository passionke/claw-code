# shellcheck shell=bash
# Host pool RPC readiness (pid + TCP). Author: kejiqing

claw_host_pool_rpc_port() {
  printf '%s' "${CLAW_POOL_DAEMON_PORT:-9943}"
}

# Fail loud when host claw-pool-daemon is not accepting RPC (gateway solve path).
claw_assert_host_pool_rpc_ready() {
  local rpc_dir="${1:?rpc_dir}"
  local port log
  port="$(claw_host_pool_rpc_port)"
  log="${rpc_dir}/daemon.log"
  local pid_file="${rpc_dir}/daemon.pid"

  if [[ ! -f "${pid_file}" ]]; then
    echo "error: host pool RPC not ready — missing ${pid_file}" >&2
    return 1
  fi
  local dpid
  dpid="$(cat "${pid_file}")"
  if ! kill -0 "${dpid}" 2>/dev/null; then
    echo "error: host pool RPC not ready — claw-pool-daemon pid ${dpid} not running" >&2
    echo "hint: tail ${log}" >&2
    return 1
  fi
  if ! python3 -c "import socket; s=socket.socket(); s.settimeout(1); s.connect(('127.0.0.1', int('${port}'))); s.close()" 2>/dev/null; then
    echo "error: host pool RPC not ready — nothing listening on 127.0.0.1:${port}" >&2
    echo "hint: tail ${log}" >&2
    return 1
  fi
  return 0
}

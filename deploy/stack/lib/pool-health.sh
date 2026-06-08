# shellcheck shell=bash
# Host pool HTTP readiness. Author: kejiqing

claw_pool_http_port() {
  printf '%s' "${CLAW_POOL_HTTP_PORT:-9944}"
}

claw_host_pool_rpc_port() {
  claw_pool_http_port
}

claw_pool_load_gateway_rpc_env() {
  local podman_dir="${1:?podman_dir}"
  if [[ -f "${podman_dir}/.claw-pool-rpc/gateway.env" ]]; then
    # shellcheck disable=SC1090
    source "${podman_dir}/.claw-pool-rpc/gateway.env"
  fi
}

claw_pool_http_base_url() {
  local podman_dir="${1:?podman_dir}"
  claw_pool_load_gateway_rpc_env "${podman_dir}"
  if [[ -n "${CLAW_POOL_HTTP_BASE:-}" ]]; then
    printf '%s' "${CLAW_POOL_HTTP_BASE}"
    return 0
  fi
  # shellcheck disable=SC1091
  source "${podman_dir}/lib/compose-include.sh"
  local host
  host="$(claw_pool_gateway_to_host_rpc_ip)" || return 1
  printf '%s' "http://${host}:$(claw_pool_http_port)"
}

claw_assert_host_pool_http_ready() {
  local rpc_dir="${1:?rpc_dir}"
  local log port
  log="${rpc_dir}/daemon.log"
  port="$(claw_pool_http_port)"

  if ! curl -fsS --connect-timeout 2 "http://127.0.0.1:${port}/healthz/live-report" >/dev/null 2>&1; then
    echo "error: host pool HTTP not ready on 127.0.0.1:${port}" >&2
    echo "hint: ./deploy/stack/gateway.sh pool-up" >&2
    echo "hint: tail ${log}" >&2
    return 1
  fi
  return 0
}

claw_assert_host_pool_rpc_ready() {
  claw_assert_host_pool_http_ready "$@"
}

claw_pool_refresh_pid_file() {
  local rpc_dir="${1:?rpc_dir}"
  local port pid
  port="$(claw_pool_http_port)"
  pid="$(lsof -nP -iTCP:"${port}" -sTCP:LISTEN -t 2>/dev/null | head -1 || true)"
  if [[ -n "${pid}" ]]; then
    printf '%s' "${pid}" >"${rpc_dir}/daemon.pid"
    printf '%s' "${pid}"
    return 0
  fi
  return 1
}

claw_gateway_container_exec() {
  local gw_ctn="${1:?container}"
  shift
  local rt
  # shellcheck disable=SC1091
  source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/compose-include.sh"
  rt="$(claw_container_runtime_cli)" || return 1
  "${rt}" exec "${gw_ctn}" "$@"
}

# True when gateway global-settings reports activeLlmConfig (claude-tap requires PG LLM). Author: kejiqing
claw_gateway_has_active_llm() {
  local port="${GATEWAY_HOST_PORT:-18088}"
  curl -fsS --connect-timeout 3 "http://127.0.0.1:${port}/v1/gateway/global-settings" 2>/dev/null \
    | python3 -c 'import json,sys; d=json.load(sys.stdin); sys.exit(0 if d.get("activeLlmConfig") else 1)' 2>/dev/null
}

claw_claude_tap_admin_host() {
  if [[ -n "${CLAUDE_TAP_DOCKER_NETWORK:-}" ]]; then
    printf '%s' "${CLAUDE_TAP_CONTAINER_NAME:-claw-claude-tap}"
    return 0
  fi
  printf '%s' "127.0.0.1"
}

# Probe + save clawTap in Admin (docker network: host=container name). Author: kejiqing
claw_claude_tap_register_in_admin() {
  local port="${GATEWAY_HOST_PORT:-18088}"
  local host proxy live probe_msg
  host="$(claw_claude_tap_admin_host)"
  proxy="${CLAUDE_TAP_PORT:-8080}"
  live="${CLAUDE_TAP_LIVE_PORT:-3000}"
  probe_msg="$(curl -fsS --connect-timeout 8 -X POST \
    "http://127.0.0.1:${port}/v1/gateway/global-settings/claw-tap/probe" \
    -H 'Content-Type: application/json' \
    -d "{\"host\":\"${host}\",\"proxyPort\":${proxy}}" 2>&1)" || {
    echo "error: clawTap probe failed (host=${host} proxyPort=${proxy}): ${probe_msg}" >&2
    [[ -n "${CLAUDE_TAP_DOCKER_NETWORK:-}" ]] && \
      echo "hint: use container name claw-claude-tap, not LAN IP (tap proxy is not on host :8080)" >&2
    return 1
  }
  if ! python3 -c 'import json,sys; d=json.loads(sys.argv[1]); sys.exit(0 if d.get("ok") else 1)' "${probe_msg}" 2>/dev/null; then
    echo "error: clawTap probe not ok: ${probe_msg}" >&2
    return 1
  fi
  curl -fsS --connect-timeout 8 -X PUT \
    "http://127.0.0.1:${port}/v1/gateway/global-settings/claw-tap" \
    -H 'Content-Type: application/json' \
    -d "{\"host\":\"${host}\",\"proxyPort\":${proxy},\"livePort\":${live}}" >/dev/null
  echo "clawTap registered in Admin: host=${host} proxyPort=${proxy} livePort=${live}"
}

claw_assert_gateway_pool_http_reachable() {
  local podman_dir="${1:?podman_dir}"
  local gw_ctn="${CLAW_GATEWAY_CONTAINER:-claw-gateway-rs}"
  local base log
  base="$(claw_pool_http_base_url "${podman_dir}")" || return 1
  log="${podman_dir}/.claw-pool-rpc/daemon.log"
  if ! claw_gateway_container_exec "${gw_ctn}" curl -fsS --connect-timeout 3 \
    "${base}/healthz/live-report" >/dev/null 2>&1; then
    echo "error: gateway cannot reach pool HTTP ${base} (Admin solve_async → 503)" >&2
    echo "hint: ./deploy/stack/gateway.sh pool-up" >&2
    echo "hint: tail ${log}" >&2
    return 1
  fi
  return 0
}

claw_assert_gateway_pool_rpc_reachable() {
  claw_assert_gateway_pool_http_reachable "$@"
}

# POST /v1/pool/rpc from gateway container (not just healthz). Retry after pack-deploy restart. Author: kejiqing
claw_wait_gateway_pool_rpc_ready() {
  local podman_dir="${1:?podman_dir}"
  local max_attempts="${2:-30}"
  local gw_ctn="${CLAW_GATEWAY_CONTAINER:-claw-gateway-rs}"
  local base body i
  base="$(claw_pool_http_base_url "${podman_dir}")" || return 1
  body='{"op":"report_state","turn_id":"connectivity-probe"}'
  for i in $(seq 1 "${max_attempts}"); do
    if claw_gateway_container_exec "${gw_ctn}" curl -fsS --max-time 8 \
      -X POST "${base}/v1/pool/rpc" \
      -H "Content-Type: application/json" \
      -d "${body}" >/dev/null 2>&1; then
      echo "gateway → pool RPC ready (attempt ${i}/${max_attempts})"
      return 0
    fi
    echo "waiting gateway → pool RPC (${i}/${max_attempts})…" >&2
    sleep 2
  done
  echo "error: gateway → pool RPC not ready after ${max_attempts} attempts (${base})" >&2
  echo "hint: tail ${podman_dir}/.claw-pool-rpc/daemon.log" >&2
  return 1
}

claw_ensure_host_pool_running() {
  local podman_dir="${1:?podman_dir}"
  local rpc_dir="${podman_dir}/.claw-pool-rpc"
  if claw_assert_host_pool_http_ready "${rpc_dir}" 2>/dev/null; then
    return 0
  fi
  echo "==> host pool down; ./deploy/stack/gateway.sh pool-up" >&2
  "${podman_dir}/lib/pool-daemon-up.sh"
  claw_assert_host_pool_http_ready "${rpc_dir}"
}

# Poll GET /readyz until clawTapCluster.consistency=strict (post-deploy tap-up race). Author: kejiqing
claw_wait_gateway_claw_tap_ready() {
  local max_attempts="${1:-45}"
  local port="${GATEWAY_HOST_PORT:-18088}"
  local i reason
  for i in $(seq 1 "${max_attempts}"); do
    if curl -fsS --connect-timeout 2 "http://127.0.0.1:${port}/readyz" >/dev/null 2>&1; then
      echo "gateway clawTap ready (/readyz attempt ${i}/${max_attempts})"
      return 0
    fi
    reason="$(curl -sS --connect-timeout 2 "http://127.0.0.1:${port}/readyz" 2>/dev/null \
      | python3 -c 'import json,sys; d=json.load(sys.stdin); print(d.get("error") or d.get("message") or d)' 2>/dev/null \
      || echo "503")"
    echo "waiting gateway /readyz (${i}/${max_attempts}): ${reason}…" >&2
    sleep 2
  done
  echo "error: gateway /readyz not strict after ${max_attempts} attempts (clawTap poll or tap-up lag)" >&2
  curl -sS "http://127.0.0.1:${port}/healthz" \
    | python3 -c 'import json,sys; print(json.dumps(json.load(sys.stdin).get("clawTapCluster"), indent=2))' >&2 \
    || true
  return 1
}

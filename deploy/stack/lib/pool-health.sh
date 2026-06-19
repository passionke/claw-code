# shellcheck shell=bash
# Host pool HTTP readiness. Author: kejiqing

_LIB_POOL_HEALTH_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=stack-instance.sh
source "${_LIB_POOL_HEALTH_DIR}/stack-instance.sh"

# Mirrors gateway `CLAW_ALLOW_RELAXED_WORKER` (default on). Author: kejiqing
claw_relaxed_worker_allowed_from_env() {
  case "${CLAW_ALLOW_RELAXED_WORKER:-true}" in
    0 | false | no | off | FALSE | NO | OFF) return 1 ;;
    *) return 0 ;;
  esac
}

# True when :port serves modern claw-sandbox (not pre-pool_outside claw-pool-daemon). kejiqing
claw_pool_port_is_modern_sandbox() {
  local port="${1:?port}"
  if curl -fsS --connect-timeout 1 "http://127.0.0.1:${port}/healthz" 2>/dev/null \
    | grep -q 'POST /v1/sandbox/rpc'; then
    return 0
  fi
  local pid cmd
  pid="$(lsof -nP -iTCP:"${port}" -sTCP:LISTEN -t 2>/dev/null | head -1 || true)"
  [[ -n "${pid}" ]] || return 1
  cmd="$(ps -p "${pid}" -o args= 2>/dev/null | head -1 || true)"
  [[ "${cmd}" == *claw-sandbox* ]]
}

# Tear down pre-pool_outside dual daemons (:9954, profile-specific launchd/systemd). kejiqing
claw_cleanup_legacy_dual_pool() {
  local podman_dir="${1:?podman_dir}"
  local rpc_root legacy_port legacy_dir
  rpc_root="$(claw_pool_rpc_root "${podman_dir}")"
  legacy_port="${CLAW_LEGACY_RELAXED_POOL_HTTP_PORT:-9954}"

  if claw_pool_port_is_modern_sandbox "${legacy_port}"; then
    echo "==> skip legacy dual-pool cleanup on :${legacy_port} (claw-sandbox listener; e.g. dev-stable on 94)" >&2
  elif curl -fsS --connect-timeout 1 "http://127.0.0.1:${legacy_port}/healthz/live-report" >/dev/null 2>&1; then
    echo "==> cleaning legacy dual-pool listener on :${legacy_port}" >&2
    # shellcheck source=nuclear-pool-reset.sh
    source "${podman_dir}/lib/nuclear-pool-reset.sh"
    claw_kill_tcp_listeners "${legacy_port}" 2>/dev/null || true
  fi

  for legacy_dir in "${rpc_root}/strict" "${rpc_root}/relaxed"; do
    [[ -d "${legacy_dir}" ]] || continue
    if [[ -f "${legacy_dir}/daemon.pid" ]]; then
      local pid
      pid="$(cat "${legacy_dir}/daemon.pid" 2>/dev/null || true)"
      [[ -n "${pid}" ]] && kill "${pid}" 2>/dev/null || true
      rm -f "${legacy_dir}/daemon.pid"
    fi
  done

  if [[ "$(uname -s)" == "Darwin" ]] && command -v launchctl >/dev/null 2>&1; then
    # shellcheck disable=SC1091
    source "${podman_dir}/lib/pool-daemon-launchd.sh"
    claw_pool_launchd_bootout_legacy_profiles
  fi

  if [[ -f "${podman_dir}/lib/pool-daemon-systemd.sh" ]]; then
    # shellcheck disable=SC1091
    source "${podman_dir}/lib/pool-daemon-systemd.sh"
    if claw_pool_use_systemd 2>/dev/null; then
      claw_pool_systemd_stop_legacy_profile_units || true
    fi
  fi
}

claw_pool_http_port() {
  printf '%s' "${CLAW_POOL_HTTP_PORT:-9944}"
}

claw_host_pool_rpc_port() {
  claw_pool_http_port
}

claw_pool_load_gateway_rpc_env() {
  local podman_dir="${1:?podman_dir}"
  local rpc_root
  rpc_root="$(claw_pool_rpc_root "${podman_dir}")"
  if [[ -f "${rpc_root}/gateway.env" ]]; then
    # shellcheck disable=SC1090
    source "${rpc_root}/gateway.env"
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

# Local dev · remote backend: probe remote HTTP + claw_pool row in shared PG. Author: kejiqing
claw_assert_remote_pool_registry_ready() {
  local podman_dir="${1:?podman_dir}"
  local remote_base pool_id
  # shellcheck disable=SC1091
  source "${podman_dir}/lib/compose-include.sh"
  remote_base="$(claw_pool_remote_base)" || {
    echo "error: CLAW_POOL_REMOTE_BASE unset" >&2
    return 1
  }
  pool_id="${CLAW_POOL_ID:-}"
  if [[ -z "${pool_id}" ]]; then
    echo "error: CLAW_POOL_ID required with CLAW_POOL_REMOTE_BASE (must match remote pool registry)" >&2
    return 1
  fi
  if ! curl -fsS --connect-timeout 5 "${remote_base}/healthz/live-report" >/dev/null 2>&1; then
    echo "error: remote pool not reachable: ${remote_base}/healthz/live-report" >&2
    return 1
  fi
  echo "remote pool HTTP ok (${remote_base})" >&2
  if ! claw_pool_registry_row_fresh "${podman_dir}"; then
    echo "error: claw_pool row stale or missing for pool_id=${pool_id} (shared PG must match remote pool)" >&2
    echo "hint: on stable host run pool-up; set CLAW_GATEWAY_DATABASE_URL to same PG" >&2
    return 1
  fi
  echo "claw_pool registry ok (pool_id=${pool_id})" >&2
  return 0
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

# True when claw_pool has a row for CLAW_POOL_ID with heartbeat < 120s (pool-daemon registry). kejiqing
claw_pool_registry_row_fresh() {
  local podman_dir="${1:?podman_dir}"
  local pool_id="${CLAW_POOL_ID:-}"
  local rt pg_ctn pg_user pg_db hb_sql hb_ok

  [[ -n "${pool_id}" ]] || return 1
  # shellcheck disable=SC1091
  source "${podman_dir}/lib/compose-include.sh"
  # shellcheck disable=SC1091
  source "${podman_dir}/lib/claw-pool-registry-env.sh"
  rt="$(claw_container_runtime_cli)" || return 1
  pg_user="${CLAW_GATEWAY_PG_USER:-claw_gateway}"
  pg_db="${CLAW_GATEWAY_PG_DATABASE:-claw_gateway}"
  pg_ctn="${CLAW_GATEWAY_PG_CONTAINER:-claw-gateway-postgres}"
  hb_sql="SELECT (EXTRACT(EPOCH FROM NOW())*1000 - last_heartbeat_ms) < 120000 FROM claw_pool WHERE pool_id='${pool_id}' LIMIT 1;"

  if claw_compose_uses_local_postgres; then
    hb_ok="$("${rt}" exec "${pg_ctn}" psql -U "${pg_user}" -d "${pg_db}" -t -A -c "${hb_sql}" 2>/dev/null | tr -d '[:space:]')"
  else
    local url pg_img
    url="$(claw_pool_daemon_database_url 2>/dev/null)" || return 1
    pg_img="${CLAW_GATEWAY_PG_IMAGE:-docker.io/library/postgres:17-alpine}"
    hb_ok="$("${rt}" run --rm "${pg_img}" psql "${url}" -t -A -c "${hb_sql}" 2>/dev/null | tr -d '[:space:]')"
  fi
  [[ "${hb_ok}" == "t" ]]
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

# Admin clawTap host: LAN IP for browser + worker; override with CLAUDE_TAP_ADMIN_HOST. kejiqing
claw_claude_tap_admin_host() {
  if [[ -n "${CLAUDE_TAP_ADMIN_HOST:-}" ]]; then
    printf '%s' "${CLAUDE_TAP_ADMIN_HOST}"
    return 0
  fi
  if [[ -n "${CLAW_POOL_ADVERTISE_HOST:-}" ]]; then
    printf '%s' "${CLAW_POOL_ADVERTISE_HOST}"
    return 0
  fi
  if [[ -n "${CLAUDE_TAP_DOCKER_NETWORK:-}" ]]; then
    printf '%s' "${CLAUDE_TAP_CONTAINER_NAME:-claw-claude-tap}"
    return 0
  fi
  printf '%s' "127.0.0.1"
}

# Probe + save clawTap in Admin (host must be reachable; publish proxy to host when using IP). kejiqing
claw_claude_tap_register_in_admin() {
  local port="${GATEWAY_HOST_PORT:-18088}"
  local host proxy live probe_msg
  host="$(claw_claude_tap_admin_host)"
  proxy="${CLAUDE_TAP_PORT:-8080}"
  live="${CLAUDE_TAP_LIVE_PORT:-3000}"
  probe_msg="$(curl -fsS --connect-timeout 8 -X POST \
    "http://127.0.0.1:${port}/v1/gateway/global-settings/claw-tap/probe" \
    -H 'Content-Type: application/json' \
    -d "{\"mode\":\"local\",\"proxyPort\":${proxy}}" 2>&1)" || {
    echo "error: clawTap probe failed (host=${host} proxyPort=${proxy}): ${probe_msg}" >&2
    echo "hint: set CLAUDE_TAP_PUBLISH_PROXY=0.0.0.0:${proxy}:${proxy} (or CLAUDE_TAP_ADMIN_HOST + published ports)" >&2
    return 1
  }
  if ! python3 -c 'import json,sys; d=json.loads(sys.argv[1]); sys.exit(0 if d.get("ok") else 1)' "${probe_msg}" 2>/dev/null; then
    echo "error: clawTap probe not ok: ${probe_msg}" >&2
    return 1
  fi
  curl -fsS --connect-timeout 8 -X PUT \
    "http://127.0.0.1:${port}/v1/gateway/global-settings/claw-tap" \
    -H 'Content-Type: application/json' \
    -d "{\"mode\":\"local\",\"livePort\":${live}}" >/dev/null
  echo "clawTap registered in Admin: mode=local livePort=${live} (proxy internal :${proxy})"
}

claw_assert_gateway_pool_http_reachable() {
  local podman_dir="${1:?podman_dir}"
  local gw_ctn="${CLAW_GATEWAY_CONTAINER:-claw-gateway-rs}"
  local base log
  base="$(claw_pool_http_base_url "${podman_dir}")" || return 1
  log="$(claw_pool_rpc_root "${podman_dir}")/daemon.log"
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
  echo "hint: tail $(claw_pool_rpc_root "${podman_dir}")/daemon.log" >&2
  return 1
}

claw_ensure_host_pool_running() {
  local podman_dir="${1:?podman_dir}"
  local rpc_dir
  rpc_dir="$(claw_pool_rpc_root "${podman_dir}")"
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

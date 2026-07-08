#!/usr/bin/env bash
# e2b-only: legacy pool-health stubs (host claw-sandbox removed). Author: kejiqing

_LIB_POOL_HEALTH_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=claw-pool-registry-env.sh
source "${_LIB_POOL_HEALTH_DIR}/claw-pool-registry-env.sh"

claw_ensure_host_pool_running() {
  return 0
}

claw_assert_host_pool_http_ready() {
  claw_ensure_host_pool_running
}

claw_assert_gateway_pool_http_reachable() {
  claw_ensure_host_pool_running
}

claw_assert_remote_pool_registry_ready() {
  claw_ensure_host_pool_running
}

# --- clawTap / LLM readiness helpers (e2b-only; not host-pool) ---
# Used by up / bootstrap-runtime / check-connectivity / admin-solve-e2e / ovs-up / observe-tap-up.

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
  echo "error: gateway /readyz not strict after ${max_attempts} attempts (clawTap poll lag)" >&2
  curl -sS "http://127.0.0.1:${port}/healthz" \
    | python3 -c 'import json,sys; print(json.dumps(json.load(sys.stdin).get("clawTapCluster"), indent=2))' >&2 \
    || true
  return 1
}

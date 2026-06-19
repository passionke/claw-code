#!/usr/bin/env bash
# Pool-scoped claude-tap: one sidecar per pool (worker network); gateway only registers/probes. Author: kejiqing

# Idempotent start after pool-daemon is up (compose network must exist for docker mode).
claw_ensure_pool_claude_tap() {
  local podman_dir="${1:?podman_dir}"
  local root_dir="${2:?root_dir}"
  # shellcheck source=/dev/null
  source "${podman_dir}/lib/claude-tap-local.sh"
  if ! claw_stack_manages_local_claude_tap; then
    return 0
  fi
  if claw_claude_tap_is_running "${podman_dir}"; then
    echo "claude-tap already running (pool-managed)" >&2
    return 0
  fi
  echo "==> pool: starting claude-tap (CLAUDE_TAP_MODE=${CLAUDE_TAP_MODE:-docker})" >&2
  claw_claude_tap_start "${podman_dir}" "${root_dir}"
  claw_claude_tap_wait_healthy 30 "${podman_dir}"
}

claw_stop_pool_claude_tap() {
  local podman_dir="${1:?podman_dir}"
  # shellcheck source=/dev/null
  source "${podman_dir}/lib/claude-tap-local.sh"
  if ! claw_stack_manages_local_claude_tap; then
    return 0
  fi
  claw_claude_tap_stop "${podman_dir}"
}

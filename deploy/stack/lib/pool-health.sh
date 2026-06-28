#!/usr/bin/env bash
# FC-only: legacy pool-health stubs (host claw-sandbox removed). Author: kejiqing

_LIB_POOL_HEALTH_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=claw-pool-registry-env.sh
source "${_LIB_POOL_HEALTH_DIR}/claw-pool-registry-env.sh"

claw_ensure_host_pool_running() {
  if claw_interactive_backend_is_fc 2>/dev/null; then
    return 0
  fi
  echo "error: host claw-sandbox pool removed; set CLAW_INTERACTIVE_BACKEND=fc" >&2
  return 1
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

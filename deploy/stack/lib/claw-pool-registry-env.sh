#!/usr/bin/env bash
# Pool registry env helpers (e2b-only; host claw-sandbox removed). Author: kejiqing

# External PG URL for verify scripts (host cannot use compose hostname `postgres`).
claw_pool_daemon_database_url() {
  if [[ -n "${CLAW_POOL_DAEMON_DATABASE_URL:-}" ]]; then
    printf '%s' "${CLAW_POOL_DAEMON_DATABASE_URL}"
    return 0
  fi
  local url="${CLAW_GATEWAY_DATABASE_URL:-}"
  if [[ -z "${url}" ]]; then
    return 1
  fi
  local port="${CLAW_GATEWAY_PG_HOST_PORT:-5433}"
  url="${url//@postgres:5432/@127.0.0.1:${port}}"
  url="${url//@postgres:/@127.0.0.1:${port}/}"
  url="${url//@claw-gateway-postgres:5432/@127.0.0.1:${port}}"
  url="${url//@claw-gateway-postgres:/@127.0.0.1:${port}/}"
  printf '%s' "${url}"
}

claw_relaxed_worker_allowed_from_env() {
  case "${CLAW_ALLOW_RELAXED_WORKER:-1}" in
    0 | false | FALSE | no | NO | off | OFF) return 1 ;;
    *) return 0 ;;
  esac
}

claw_default_pool_id() {
  if [[ -n "${CLAW_POOL_ID:-}" ]]; then
    printf '%s\n' "${CLAW_POOL_ID}"
    return 0
  fi
  local host
  host="$(hostname -s 2>/dev/null || hostname 2>/dev/null || echo localhost)"
  printf 'pool-%s\n' "${host}"
}

claw_export_pool_registry_env() {
  local rpc_root="${1:?}"
  mkdir -p "${rpc_root}"
  {
    printf '%s\n' '# GENERATED — e2b-only pool id (no host claw-sandbox). kejiqing'
    printf '%s\n' "CLAW_POOL_ID=${CLAW_POOL_ID:-$(claw_default_pool_id)}"
  } >"${rpc_root}/pool-registry.env"
}

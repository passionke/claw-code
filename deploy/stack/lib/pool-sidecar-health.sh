# shellcheck shell=bash
# Compose claw-pool-daemon sidecar: deploy contract + runtime readiness. Author: kejiqing

claw_pool_sidecar_container() {
  printf '%s' "${CLAW_POOL_DAEMON_CONTAINER:-claw-pool-daemon}"
}

claw_gateway_container() {
  printf '%s' "${CLAW_GATEWAY_CONTAINER:-claw-gateway-rs}"
}

# Sidecar mode must dial the compose service name, not LAN IP / host.docker.internal.
claw_pool_sidecar_rpc_host() {
  printf '%s' claw-pool-daemon
}

claw_warn_ignored_pool_tcp_host_override() {
  local want="$1"
  local set="${CLAW_POOL_DAEMON_TCP_HOST:-}"
  [[ -n "${set}" && "${set}" != "${want}" ]] || return 0
  echo "warn: CLAW_POOL_DAEMON_TCP_HOST=${set} ignored for compose pool sidecar; using ${want}" >&2
  echo "      (CLAW_POOL_ADVERTISE_HOST is for worker/SSE advertise only, not gateway→pool RPC)" >&2
}

# Static contract: privileged + host docker CLI (Engine ≥29 needs API ≥1.44). Author: kejiqing
claw_assert_pool_sidecar_compose_contract() {
  local stack_dir="${1:?}"
  local yml="${stack_dir}/podman-compose.pool-rpc.yml"
  [[ -f "${yml}" ]] || {
    echo "error: missing ${yml} (compose pool sidecar)" >&2
    return 1
  }
  if ! grep -Eq 'privileged:[[:space:]]*true' "${yml}"; then
    echo "error: ${yml} must set privileged: true for docker_pool mount --make-rshared" >&2
    return 1
  fi
  if ! grep -q '/usr/bin/docker:/usr/bin/docker' "${yml}"; then
    echo "error: ${yml} must bind-mount host /usr/bin/docker (image docker.io is API 1.41; Engine ≥29 needs ≥1.44)" >&2
    return 1
  fi
  if ! grep -q 'propagation: shared' "${yml}"; then
    echo "error: ${yml} must bind-mount work_root with propagation: shared (pool inject → worker rslave)" >&2
    return 1
  fi
  return 0
}

# Compare dotted API versions (e.g. 1.44 >= 1.41).
claw_api_version_ge() {
  python3 - "$1" "$2" <<'PY'
import sys

def parse(v: str) -> tuple[int, int]:
    parts = v.strip().split(".", 1)
    if len(parts) != 2:
        raise ValueError(v)
    return int(parts[0]), int(parts[1])

a = parse(sys.argv[1])
b = parse(sys.argv[2])
sys.exit(0 if a >= b else 1)
PY
}

claw_assert_pool_container_docker_cli() {
  local rt ctn min_client="${CLAW_POOL_DOCKER_MIN_CLIENT_API:-1.44}"
  rt="$(claw_container_runtime_cli)" || return 1
  ctn="$(claw_pool_sidecar_container)"
  if ! "${rt}" ps --format '{{.Names}}' | grep -qx "${ctn}"; then
    echo "error: pool sidecar container ${ctn} not running" >&2
    return 1
  fi
  local client_ver server_min
  client_ver="$("${rt}" exec "${ctn}" docker version -f '{{.Client.APIVersion}}' 2>/dev/null || true)"
  if [[ -z "${client_ver}" ]]; then
    if ! "${rt}" exec "${ctn}" docker ps -q --limit 1 >/dev/null 2>&1; then
      local err
      err="$("${rt}" exec "${ctn}" docker ps -q --limit 1 2>&1 || true)"
      echo "error: pool sidecar docker CLI probe failed: ${err}" >&2
      echo "hint: ensure podman-compose.pool-rpc.yml bind-mounts host /usr/bin/docker" >&2
      return 1
    fi
    return 0
  fi
  if ! claw_api_version_ge "${client_ver}" "${min_client}"; then
    echo "error: pool sidecar docker client API ${client_ver} < required ${min_client}" >&2
    return 1
  fi
  server_min="$("${rt}" exec "${ctn}" docker version -f '{{.Server.MinAPIVersion}}' 2>/dev/null || true)"
  if [[ -n "${server_min}" ]] && ! claw_api_version_ge "${client_ver}" "${server_min}"; then
    echo "error: pool sidecar docker client ${client_ver} < engine minimum ${server_min}" >&2
    return 1
  fi
  return 0
}

claw_assert_gateway_pool_rpc_reachable() {
  local rt gw port rpc_host
  rt="$(claw_container_runtime_cli)" || return 1
  gw="$(claw_gateway_container)"
  port="${CLAW_POOL_DAEMON_PORT:-9943}"
  rpc_host="$(claw_pool_sidecar_rpc_host)"
  if ! "${rt}" ps --format '{{.Names}}' | grep -qx "${gw}"; then
    echo "error: gateway container ${gw} not running" >&2
    return 1
  fi
  if ! "${rt}" exec "${gw}" timeout 2 bash -c "echo > /dev/tcp/${rpc_host}/${port}" 2>/dev/null; then
    echo "error: gateway cannot reach ${rpc_host}:${port} (check .claw-pool-rpc/gateway.env / CLAW_POOL_DAEMON_TCP_HOST)" >&2
    return 1
  fi
  return 0
}

claw_assert_gateway_pool_rpc_env() {
  local env_file="${1:?}/.claw-pool-rpc/gateway.env"
  local want port line host_port
  want="$(claw_pool_sidecar_rpc_host):${CLAW_POOL_DAEMON_PORT:-9943}"
  [[ -f "${env_file}" ]] || {
    echo "error: missing ${env_file} — run gateway.sh up" >&2
    return 1
  }
  line="$(grep -E '^CLAW_POOL_DAEMON_TCP=' "${env_file}" | tail -1 || true)"
  host_port="${line#CLAW_POOL_DAEMON_TCP=}"
  if [[ "${host_port}" != "${want}" ]]; then
    echo "error: ${env_file} has CLAW_POOL_DAEMON_TCP=${host_port:-<unset>}, want ${want}" >&2
    echo "hint: unset CLAW_POOL_DAEMON_TCP_HOST in .env (do not use LAN IP for compose sidecar)" >&2
    return 1
  fi
  return 0
}

claw_assert_pool_warm_worker() {
  local rt tries="${CLAW_POOL_WARM_WAIT_TRIES:-60}"
  rt="$(claw_container_runtime_cli)" || return 1
  for _ in $(seq 1 "${tries}"); do
    if "${rt}" ps --filter "name=claw-worker" --format '{{.Names}}' 2>/dev/null | grep -q .; then
      return 0
    fi
    sleep 0.5
  done
  echo "error: no claw-worker container after pool warm (${tries} tries)" >&2
  echo "hint: docker logs claw-pool-daemon | tail -40" >&2
  return 1
}

# End-to-end: pool inject bind must appear under worker /claw_host_root (rshared + rslave). Author: kejiqing
claw_assert_pool_bind_propagation_e2e() {
  local stack_dir="${1:?}"
  local rt pool worker work_root guest probe_session probe_marker
  rt="$(claw_container_runtime_cli)" || return 1
  pool="$(claw_pool_sidecar_container)"
  worker="$("${rt}" ps --filter "name=claw-worker" --format '{{.Names}}' 2>/dev/null | head -1)"
  if [[ -z "${worker}" ]]; then
    echo "error: bind propagation e2e: no claw-worker (warm pool first)" >&2
    return 1
  fi
  work_root="${CLAW_POOL_WORK_ROOT_BIND_SRC:-${stack_dir}/claw-workspace}"
  guest="${work_root}/.claw-pool-slot/slot-0/guest"
  probe_session="${work_root}/.claw-propagation-probe-session"
  probe_marker="${probe_session}/gateway-solve-task.json"
  mkdir -p "${guest}" "${probe_session}"
  printf '%s\n' '{"probe":true}' >"${probe_marker}"
  # Remount session tree into guest (same as pool acquire inject).
  if ! "${rt}" exec "${pool}" mount --bind "${probe_session}" "${guest}" 2>/dev/null; then
    echo "error: pool container could not mount --bind session → guest (privileged + work_root rshared?)" >&2
    return 1
  fi
  if ! "${rt}" exec "${worker}" test -f /claw_host_root/gateway-solve-task.json 2>/dev/null; then
    echo "error: worker ${worker} does not see injected file at /claw_host_root/gateway-solve-task.json (bind propagation broken)" >&2
    "${rt}" exec "${pool}" umount "${guest}" 2>/dev/null || true
    return 1
  fi
  "${rt}" exec "${pool}" umount "${guest}" 2>/dev/null || true
  rm -rf "${probe_session}"
  return 0
}

claw_assert_pool_sidecar_ready() {
  local stack_dir="${1:?}"
  claw_assert_pool_sidecar_compose_contract "${stack_dir}" || return 1
  claw_assert_gateway_pool_rpc_env "${stack_dir}" || return 1
  claw_assert_pool_container_docker_cli || return 1
  claw_assert_gateway_pool_rpc_reachable || return 1
  claw_assert_pool_warm_worker || return 1
  claw_assert_pool_bind_propagation_e2e "${stack_dir}" || return 1
  return 0
}

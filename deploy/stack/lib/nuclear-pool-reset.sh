# shellcheck shell=bash
# Stop pool daemon, free TCP port, remove every claw worker container (any name/tag). Author: kejiqing

claw_kill_tcp_listeners() {
  local port="$1"
  if command -v fuser >/dev/null 2>&1; then
    fuser -k "${port}/tcp" 2>/dev/null || true
    sleep 0.3
    return 0
  fi
  if command -v ss >/dev/null 2>&1; then
    local pid
    while read -r pid; do
      [[ -n "${pid}" ]] || continue
      kill "${pid}" 2>/dev/null || true
    done < <(ss -ltnp "sport = :${port}" 2>/dev/null | sed -n 's/.*pid=\([0-9]*\).*/\1/p' | sort -u)
    sleep 0.3
  fi
}

claw_remove_all_gateway_workers() {
  local rt
  rt="$(claw_container_runtime_cli)" || return 0
  local ids_w ids_g ids_by_image ids
  ids_w="$("${rt}" ps -aq --filter name='claw-worker-' 2>/dev/null || true)"
  ids_g="$("${rt}" ps -aq --filter name='claw-gw-' 2>/dev/null || true)"
  ids_by_image="$(
    "${rt}" ps -a --format '{{.ID}} {{.Image}}' 2>/dev/null \
      | awk '/claw-gateway-worker/ { print $1 }' \
      | sort -u \
      | tr '\n' ' '
  )"
  ids="${ids_w} ${ids_g} ${ids_by_image}"
  if [[ -z "${ids//[$'\t\r\n ']}" ]]; then
    return 0
  fi
  echo "Removing all claw-gateway-worker containers…" >&2
  # shellcheck disable=SC2086
  "${rt}" rm -f ${ids} 2>/dev/null || true
}

# Full pool teardown before a release `up` (daemon + workers; does not remove gateway compose). kejiqing
claw_nuclear_pool_reset() {
  local podman_dir="$1"
  local port="${CLAW_POOL_DAEMON_PORT:-9943}"
  echo "==> nuclear pool reset (release up): stop daemon, free :${port}, remove all workers" >&2
  "${podman_dir}/lib/pool-daemon-down.sh" 2>/dev/null || true
  claw_kill_tcp_listeners "${port}"
  claw_remove_all_gateway_workers
}

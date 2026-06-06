# shellcheck shell=bash
# Stop pool daemon, free TCP port, remove every claw worker container (any name/tag). Author: kejiqing

claw_kill_tcp_listeners() {
  local port="$1"
  [[ -n "${port}" ]] || return 0
  local pid
  if command -v lsof >/dev/null 2>&1; then
    while read -r pid; do
      [[ -n "${pid}" ]] || continue
      kill "${pid}" 2>/dev/null || true
    done < <(lsof -nP -iTCP:"${port}" -sTCP:LISTEN -t 2>/dev/null || true)
    sleep 0.3
    return 0
  fi
  if command -v ss >/dev/null 2>&1; then
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

claw_lazy_umount() {
  local target="$1"
  [[ -n "${target}" ]] || return 0
  umount -l "${target}" 2>/dev/null \
    || sudo -n umount -l "${target}" 2>/dev/null \
    || sudo umount -l "${target}" 2>/dev/null \
    || true
}

# v1.4.7 bind-mount slots leave guest/ mounts after worker rm; v1.4.9 uses tmpfs — tear down legacy mounts first. kejiqing
claw_teardown_pool_slot_mounts() {
  local slot_root="$1"
  local guest target
  [[ -d "${slot_root}" ]] || return 0

  for guest in "${slot_root}"/slot-*/guest; do
    [[ -e "${guest}" ]] || continue
    claw_lazy_umount "${guest}"
  done

  if command -v findmnt >/dev/null 2>&1; then
    while read -r target; do
      [[ -n "${target}" && "${target}" != "${slot_root}" ]] || continue
      claw_lazy_umount "${target}"
    done < <(findmnt -R -n -o TARGET "${slot_root}" 2>/dev/null | awk '{ print length, $0 }' | sort -rn | cut -d' ' -f2-)
  fi
}

claw_remove_pool_slot_tree() {
  local slot_root="$1"
  [[ -d "${slot_root}" ]] || return 0
  claw_teardown_pool_slot_mounts "${slot_root}"
  rm -rf "${slot_root}" 2>/dev/null \
    || sudo -n rm -rf "${slot_root}" 2>/dev/null \
    || sudo rm -rf "${slot_root}" || {
      echo "error: cannot remove ${slot_root} (mount still busy). Check: findmnt | grep claw-pool-slot" >&2
      return 1
    }
}

# Full pool teardown before a release `up` (daemon + workers; does not remove gateway compose). kejiqing
claw_nuclear_pool_reset() {
  local podman_dir="$1"
  local port="${CLAW_POOL_DAEMON_PORT:-9943}"
  local http_port="${CLAW_POOL_HTTP_PORT:-9944}"
  echo "==> nuclear pool reset (release up): stop daemon, free :${port}/:${http_port}, remove all workers" >&2
  "${podman_dir}/lib/pool-daemon-down.sh" 2>/dev/null || true
  claw_kill_tcp_listeners "${port}"
  claw_kill_tcp_listeners "${http_port}"
  claw_remove_all_gateway_workers
  local work_root="${CLAW_POOL_WORK_ROOT_BIND_SRC:-${podman_dir}/claw-workspace}"
  local slot_root="${work_root}/${CLAW_POOL_SLOT_DIR:-.claw-pool-slot}"
  if [[ -d "${slot_root}" ]]; then
    echo "==> remove pool slot mount tree ${slot_root} (avoid stale root-owned guests)" >&2
    claw_remove_pool_slot_tree "${slot_root}"
  fi
}

# shellcheck shell=bash
# Stop pool daemon, free TCP port, remove every claw worker container (any name/tag). Author: kejiqing

_LIB_NUCLEAR_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck disable=SC1091
source "${_LIB_NUCLEAR_DIR}/log-ts.sh"

claw_docker_op_timeout_sec() {
  printf '%s' "${CLAW_DOCKER_OP_TIMEOUT_SEC:-60}"
}

# GNU timeout on Linux CI; no-op when absent (macOS dev). kejiqing
claw_run_with_timeout() {
  local secs="$1"
  shift
  if command -v timeout >/dev/null 2>&1; then
    timeout --kill-after=5 "${secs}" "$@"
  else
    "$@"
  fi
}

claw_pool_http_health_alive() {
  local port="${1:?port}"
  curl -fsS --connect-timeout 1 --max-time 2 \
    "http://127.0.0.1:${port}/healthz/live-report" >/dev/null 2>&1
}

claw_tcp_port_listening() {
  local port="$1"
  [[ -n "${port}" ]] || return 1
  if command -v lsof >/dev/null 2>&1; then
    lsof -nP -iTCP:"${port}" -sTCP:LISTEN -t >/dev/null 2>&1
    return $?
  fi
  if command -v ss >/dev/null 2>&1; then
    ss -ltn "sport = :${port}" 2>/dev/null | grep -q LISTEN
    return $?
  fi
  return 1
}

claw_tcp_listen_pids() {
  local port="$1" pid
  if command -v lsof >/dev/null 2>&1; then
    lsof -nP -iTCP:"${port}" -sTCP:LISTEN -t 2>/dev/null | sort -u | tr '\n' ' '
    return 0
  fi
  if command -v ss >/dev/null 2>&1; then
    ss -ltnp "sport = :${port}" 2>/dev/null | sed -n 's/.*pid=\([0-9]*\).*/\1/p' | sort -u | tr '\n' ' '
  fi
}

claw_print_tcp_port_status() {
  local port="$1" label="${2:-}"
  local listening health pids
  listening=no
  claw_tcp_port_listening "${port}" && listening=yes
  health=no
  claw_pool_http_health_alive "${port}" && health=yes
  pids="$(claw_tcp_listen_pids "${port}")"
  pids="${pids//[$'\t\r\n ']/}"
  if [[ -n "${label}" ]]; then
    claw_log "TCP :${port} (${label}) listening=${listening} pool_health=${health} listen_pids=${pids:-none}"
  else
    claw_log "TCP :${port} listening=${listening} pool_health=${health} listen_pids=${pids:-none}"
  fi
}

claw_kill_pids_on_tcp_port() {
  local port="$1" signal="${2:-TERM}"
  local pid
  if command -v lsof >/dev/null 2>&1; then
    while read -r pid; do
      [[ -n "${pid}" ]] || continue
      kill "-${signal}" "${pid}" 2>/dev/null \
        || sudo -n kill "-${signal}" "${pid}" 2>/dev/null \
        || true
    done < <(lsof -nP -iTCP:"${port}" -sTCP:LISTEN -t 2>/dev/null || true)
    return 0
  fi
  if command -v ss >/dev/null 2>&1; then
    while read -r pid; do
      [[ -n "${pid}" ]] || continue
      kill "-${signal}" "${pid}" 2>/dev/null \
        || sudo -n kill "-${signal}" "${pid}" 2>/dev/null \
        || true
    done < <(ss -ltnp "sport = :${port}" 2>/dev/null | sed -n 's/.*pid=\([0-9]*\).*/\1/p' | sort -u)
  fi
}

# Kill listeners on :port; docker --pid=host fallback when root-owned (systemd pool on CI). kejiqing
claw_kill_tcp_listeners_privileged() {
  local port="$1" rt image lib_dir
  [[ -n "${port}" ]] || return 0
  lib_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
  # shellcheck disable=SC1091
  source "${lib_dir}/compose-include.sh"
  rt="$(claw_container_runtime_cli)" || return 1
  image="${CONTAINER_BASE_REGISTRY:-docker.1ms.run}/library/alpine:3.20"
  local secs
  secs="$(claw_docker_op_timeout_sec)"
  claw_log "kill TCP :${port} via ${rt} (privileged pid=host, timeout=${secs}s)"
  if ! claw_run_with_timeout "${secs}" "${rt}" run --rm --pid=host --privileged "${image}" sh -c "
    apk add --no-cache lsof >/dev/null 2>&1 || true
    for pid in \$(lsof -nP -iTCP:${port} -sTCP:LISTEN -t 2>/dev/null); do
      kill -9 \"\$pid\" 2>/dev/null || true
    done
  "; then
    claw_log "warning: privileged ${rt} run timed out after ${secs}s (TCP :${port})"
  fi
}

claw_kill_tcp_listeners() {
  local port="$1" label="${2:-}"
  [[ -n "${port}" ]] || return 0
  claw_print_tcp_port_status "${port}" "${label} before"
  local t0=$SECONDS
  claw_kill_pids_on_tcp_port "${port}" TERM
  sleep 0.3
  if claw_tcp_port_listening "${port}"; then
    claw_kill_pids_on_tcp_port "${port}" 9
    sleep 0.2
  fi
  if claw_tcp_port_listening "${port}" || claw_pool_http_health_alive "${port}"; then
    claw_kill_tcp_listeners_privileged "${port}"
    sleep 0.5
  fi
  if claw_pool_http_health_alive "${port}"; then
    claw_kill_tcp_listeners_privileged "${port}"
    sleep 0.5
  fi
  claw_print_tcp_port_status "${port}" "${label} after"
  claw_log "TCP :${port} kill done in $((SECONDS - t0))s"
}

claw_count_docker_ids() {
  awk 'NF { c++ } END { print c+0 }'
}

claw_unique_docker_ids() {
  awk 'NF' | sort -u | tr '\n' ' '
}

claw_remove_all_gateway_workers() {
  local rt lib_dir t0 t_ps t_rm
  lib_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
  # pack-deploy sources this file without compose-include.sh; load runtime CLI here. kejiqing
  # shellcheck disable=SC1091
  source "${lib_dir}/compose-include.sh"
  rt="$(claw_container_runtime_cli)" || return 1
  local ids_w ids_g ids_by_image ids n_w n_g n_img n_unique secs
  secs="$(claw_docker_op_timeout_sec)"
  claw_log "worker inventory: ${rt} ps -a (claw-worker / claw-gw / claw-gateway-worker image, timeout=${secs}s) …"
  t0=$SECONDS
  t_ps=$SECONDS
  ids_w="$(claw_run_with_timeout "${secs}" "${rt}" ps -aq --filter name='claw-worker-' 2>/dev/null || true)"
  ids_g="$(claw_run_with_timeout "${secs}" "${rt}" ps -aq --filter name='claw-gw-' 2>/dev/null || true)"
  ids_by_image="$(
    claw_run_with_timeout "${secs}" "${rt}" ps -a --format '{{.ID}} {{.Image}}' 2>/dev/null \
      | awk '/claw-gateway-worker/ { print $1 }' \
      | sort -u \
      | tr '\n' ' '
  )"
  claw_log "worker inventory: ${rt} ps done in $((SECONDS - t_ps))s"
  n_w="$(printf '%s\n' ${ids_w} | claw_count_docker_ids)"
  n_g="$(printf '%s\n' ${ids_g} | claw_count_docker_ids)"
  n_img="$(printf '%s\n' ${ids_by_image} | claw_count_docker_ids)"
  ids="$(printf '%s\n%s\n%s\n' ${ids_w} ${ids_g} ${ids_by_image} | claw_unique_docker_ids)"
  n_unique="$(printf '%s\n' ${ids} | claw_count_docker_ids)"
  claw_log "worker inventory: claw-worker-*=${n_w} claw-gw-*=${n_g} image~claw-gateway-worker=${n_img} unique_rm=${n_unique}"
  if [[ "${n_unique}" == "0" ]]; then
    claw_log "worker inventory: nothing to remove (${rt} rm skipped)"
    return 0
  fi
  claw_log "${rt} rm -f ${n_unique} worker container(s) (timeout=${secs}s) …"
  t_rm=$SECONDS
  # shellcheck disable=SC2086
  if ! claw_run_with_timeout "${secs}" "${rt}" rm -f ${ids} 2>&1 | head -20 >&2; then
    claw_log "warning: ${rt} rm -f timed out or failed after ${secs}s"
  fi
  claw_log "${rt} rm -f done in $((SECONDS - t_rm))s (inventory total $((SECONDS - t0))s)"
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
  local t0=$SECONDS t_step
  claw_log "nuclear pool reset begin: daemon :${port}/:${http_port} + workers + slot tree"
  claw_log "[1/4] pool-daemon-down (skip tcp/legacy; nuclear owns teardown) …"
  t_step=$SECONDS
  CLAW_POOL_DOWN_TCP_KILL=0 CLAW_POOL_DOWN_LEGACY_CLEANUP=0 \
    "${podman_dir}/lib/pool-daemon-down.sh" || true
  claw_log "[1/4] pool-daemon-down done in $((SECONDS - t_step))s"
  claw_log "[2/4] free TCP :${port} …"
  t_step=$SECONDS
  claw_kill_tcp_listeners "${port}" "pool-rpc"
  claw_log "[2/4] free TCP :${port} done in $((SECONDS - t_step))s"
  claw_log "[3/4] free TCP :${http_port} …"
  t_step=$SECONDS
  claw_kill_tcp_listeners "${http_port}" "pool-http"
  claw_log "[3/4] free TCP :${http_port} done in $((SECONDS - t_step))s"
  claw_log "[4/4] remove workers …"
  t_step=$SECONDS
  claw_remove_all_gateway_workers
  claw_log "[4/4] remove workers done in $((SECONDS - t_step))s"
  local work_root="${CLAW_POOL_WORK_ROOT_BIND_SRC:-${podman_dir}/claw-workspace}"
  local slot_root="${work_root}/${CLAW_POOL_SLOT_DIR:-.claw-pool-slot}"
  if [[ -d "${slot_root}" ]]; then
    claw_log "remove pool slot mount tree ${slot_root} …"
    t_step=$SECONDS
    claw_remove_pool_slot_tree "${slot_root}"
    claw_log "remove pool slot mount tree done in $((SECONDS - t_step))s"
  else
    claw_log "pool slot tree absent (${slot_root}); skip"
  fi
  claw_log "nuclear pool reset done in $((SECONDS - t0))s"
}

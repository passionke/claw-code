#!/usr/bin/env bash
# GitHub Actions CI: claw-sandbox in Docker (survives job orphan cleanup). Author: kejiqing
set -euo pipefail

claw_pool_docker_container_name() {
  printf '%s' "${CLAW_POOL_DOCKER_CONTAINER_NAME:-claw-sandbox}"
}

claw_pool_use_docker_supervisor() {
  case "${CLAW_POOL_SUPERVISOR:-}" in
    docker) ;;
    *) return 1 ;;
  esac
  # shellcheck disable=SC1091
  source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/compose-include.sh"
  [[ "$(claw_container_runtime_cli 2>/dev/null || true)" == docker ]]
}

claw_pool_docker_runtime() {
  # shellcheck disable=SC1091
  source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/compose-include.sh"
  claw_container_runtime_cli
}

claw_pool_write_docker_env_file() {
  local src="$1" dst="$2"
  local k v
  : >"${dst}"
  while IFS= read -r line || [[ -n "${line}" ]]; do
    [[ "${line}" =~ ^[A-Za-z_][A-Za-z0-9_]*= ]] || continue
    k="${line%%=*}"
    v="${line#*=}"
    if [[ "${v}" =~ ^\'.*\'$ ]]; then
      v="${v:1:${#v}-2}"
      v="${v//\'\\\'\'/\'}"
    fi
    printf '%s=%s\n' "${k}" "${v}" >>"${dst}"
  done <"${src}"
}

claw_pool_docker_pool_image() {
  local base_reg="${CONTAINER_BASE_REGISTRY:-docker.1ms.run}"
  if [[ -n "${CLAW_POOL_DOCKER_IMAGE:-}" ]]; then
    printf '%s' "${CLAW_POOL_DOCKER_IMAGE}"
    return 0
  fi
  if [[ -n "${CLAW_DOCKER_IMAGE:-}" ]]; then
    printf '%s' "${CLAW_DOCKER_IMAGE}"
    return 0
  fi
  if [[ -n "${CLAW_PODMAN_IMAGE:-}" ]]; then
    printf '%s' "${CLAW_PODMAN_IMAGE}"
    return 0
  fi
  printf '%s' "${base_reg}/library/debian:bookworm-slim"
}

claw_pool_docker_dump_logs() {
  local rt ctr
  rt="$(claw_pool_docker_runtime 2>/dev/null || true)"
  ctr="$(claw_pool_docker_container_name)"
  [[ -n "${rt}" ]] || return 0
  echo "==> pool-daemon-docker: inspect ${ctr}" >&2
  "${rt}" inspect "${ctr}" 2>&1 | tail -40 >&2 || true
  echo "==> pool-daemon-docker: logs ${ctr} (tail 80)" >&2
  "${rt}" logs --tail 80 "${ctr}" 2>&1 >&2 || true
}

claw_pool_docker_stop() {
  local rt ctr
  rt="$(claw_pool_docker_runtime 2>/dev/null || true)"
  [[ -n "${rt}" ]] || return 0
  ctr="$(claw_pool_docker_container_name)"
  if "${rt}" inspect "${ctr}" >/dev/null 2>&1; then
    echo "==> pool-daemon-docker: removing ${ctr}" >&2
    "${rt}" rm -f "${ctr}" 2>/dev/null || true
  fi
}

claw_pool_docker_running() {
  local rt ctr
  rt="$(claw_pool_docker_runtime 2>/dev/null || true)"
  [[ -n "${rt}" ]] || return 1
  ctr="$(claw_pool_docker_container_name)"
  "${rt}" inspect -f '{{.State.Running}}' "${ctr}" 2>/dev/null | grep -q true
}

# Start claw-sandbox in Docker (host network + docker.sock). Survives GHA job teardown.
claw_pool_docker_up() {
  local rpc_dir="$1" bin="$2" repo_root="$3" work_root="$4"
  local rt ctr image docker_env docker_bin
  rt="$(claw_pool_docker_runtime)"
  ctr="$(claw_pool_docker_container_name)"
  image="$(claw_pool_docker_pool_image)"
  docker_env="${rpc_dir}/pool-daemon.docker.env"
  docker_bin="$(command -v docker 2>/dev/null || true)"
  if [[ -z "${docker_bin}" || ! -x "${docker_bin}" ]]; then
    echo "error: pool-daemon-docker: host docker CLI not found in PATH" >&2
    return 1
  fi

  claw_pool_docker_stop
  "${rt}" pull "${image}" >/dev/null 2>&1 || true
  claw_pool_write_docker_env_file "${rpc_dir}/pool-daemon.env" "${docker_env}"
  printf '%s=%s\n' "CLAW_POOL_DAEMON_BIN" "/usr/local/bin/claw-sandbox" >>"${docker_env}"

  if ! "${rt}" run -d \
    --name "${ctr}" \
    --restart unless-stopped \
    --network host \
    --user root \
    --entrypoint /usr/local/bin/claw-sandbox \
    -v /var/run/docker.sock:/var/run/docker.sock \
    -v "${docker_bin}:/usr/local/bin/docker:ro" \
    -v "${work_root}:${work_root}" \
    -v "${repo_root}:${repo_root}" \
    -v "${bin}:/usr/local/bin/claw-sandbox:ro" \
    --env-file "${docker_env}" \
    "${image}"; then
    echo "error: pool-daemon-docker: docker run failed (image=${image})" >&2
    claw_pool_docker_dump_logs
    return 1
  fi

  echo "==> pool-daemon-docker: started ${ctr} (image=${image})" >&2
}

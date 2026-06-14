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

claw_pool_docker_env_args() {
  local env_file="$1"
  local k v
  [[ -f "${env_file}" ]] || return 0
  while IFS= read -r line || [[ -n "${line}" ]]; do
    [[ "${line}" =~ ^[A-Za-z_][A-Za-z0-9_]*= ]] || continue
    k="${line%%=*}"
    v="${line#*=}"
    if [[ "${v}" =~ ^\'.*\'$ ]]; then
      v="${v:1:${#v}-2}"
      v="${v//\'\\\'\'/\'}"
    fi
    printf '%s\0%s\0' -e "${k}=${v}"
  done <"${env_file}"
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
  local rt ctr base_reg image env_file args=() pair
  rt="$(claw_pool_docker_runtime)"
  ctr="$(claw_pool_docker_container_name)"
  base_reg="${CONTAINER_BASE_REGISTRY:-docker.1ms.run}"
  image="${CLAW_POOL_DOCKER_IMAGE:-${base_reg}/library/debian:bookworm-slim}"
  env_file="${rpc_dir}/pool-daemon.env"

  claw_pool_docker_stop
  "${rt}" pull "${image}" >/dev/null 2>&1 || true

  while IFS= read -r -d '' pair; do
    args+=("${pair}")
  done < <(claw_pool_docker_env_args "${env_file}")

  "${rt}" run -d \
    --name "${ctr}" \
    --restart unless-stopped \
    --network host \
    -v /var/run/docker.sock:/var/run/docker.sock \
    -v "${work_root}:${work_root}" \
    -v "${repo_root}:${repo_root}" \
    -v "${bin}:/usr/local/bin/claw-sandbox:ro" \
    "${args[@]}" \
    -e "CLAW_POOL_DAEMON_BIN=/usr/local/bin/claw-sandbox" \
    "${image}" \
    /usr/local/bin/claw-sandbox

  echo "==> pool-daemon-docker: started ${ctr} (image=${image})" >&2
}

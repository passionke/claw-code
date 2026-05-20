#!/usr/bin/env bash
# One-shot host checks before `gateway.sh up` (rootless podman / legacy docker-compose). Author: kejiqing
set -euo pipefail

claw_deploy_preflight() {
  local podman_dir="${1:?}"
  # shellcheck disable=SC1091
  source "${podman_dir}/lib/compose-include.sh"

  echo "==> deploy preflight" >&2
  local rt sock
  rt="$(claw_container_runtime_cli)" || return 1
  echo "    runtime=${rt}" >&2

  if ! "${rt}" info >/dev/null 2>&1; then
    echo "error: ${rt} info failed" >&2
    return 1
  fi

  sock="$(claw_container_socket_path)" || return 1
  export CLAW_CONTAINER_SOCKET="${sock}"
  export DOCKER_HOST="unix://${sock}"
  if [[ ! -S "${sock}" ]] || [[ ! -r "${sock}" || ! -w "${sock}" ]]; then
    echo "error: cannot use container socket: ${sock}" >&2
    echo "hint: podman info --format '{{.Host.RemoteSocket.Path}}' → set CLAW_CONTAINER_SOCKET in .env" >&2
    return 1
  fi
  echo "    socket=${sock}" >&2

  export COMPOSE_PROJECT_NAME="${COMPOSE_PROJECT_NAME:-claw}"
  local net="${COMPOSE_PROJECT_NAME}_default"
  claw_network_ensure "${rt}" "${net}"
  claw_network_ensure "${rt}" "stack_default"

  local pg="${CLAW_GATEWAY_PG_IMAGE:-docker.io/library/postgres:17-alpine}"
  if ! "${rt}" image exists "${pg}" >/dev/null 2>&1; then
    echo "    pull ${pg} …" >&2
    "${rt}" pull "${pg}"
  fi

  for v in CLAW_PROJECTS_GIT_URL CLAW_PROJECTS_GIT_BRANCH CLAW_PROJECTS_GIT_AUTHOR; do
    if [[ -z "${!v:-}" ]]; then
      echo "error: ${v} is required in .env (gateway will exit without it)" >&2
      return 1
    fi
  done

  echo "==> preflight ok (compose project=${COMPOSE_PROJECT_NAME}, network=${net})" >&2
}

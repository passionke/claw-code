# shellcheck shell=bash
# Shared Podman Compose -f list when CLAW_SOLVE_ISOLATION=podman_pool. Source from deploy/podman/*.sh.
# Sets CLAW_PODMAN_COMPOSE_ARGS and CLAW_POOL_WORK_ROOT_HOST. Author: kejiqing

claw_podman_export_pool_workspace() {
  local script_dir="$1"
  mkdir -p "${script_dir}/claw-workspace"
  export CLAW_POOL_WORK_ROOT_HOST="$(cd "${script_dir}" && pwd)/claw-workspace"
}

claw_podman_load_compose_args() {
  local script_dir="$1"
  local env_file="$2"
  CLAW_PODMAN_COMPOSE_ARGS=( -f "${script_dir}/podman-compose.yml" )
  if [[ ! -f "${env_file}" ]]; then
    return 0
  fi
  set -a
  # shellcheck disable=SC1090
  source "${env_file}"
  set +a
  if [[ "${CLAW_SOLVE_ISOLATION:-inprocess}" == "podman_pool" ]]; then
    if [[ -z "${PODMAN_HOST_SOCK:-}" ]]; then
      echo "error: CLAW_SOLVE_ISOLATION=podman_pool requires PODMAN_HOST_SOCK (host Podman API socket)." >&2
      echo "example (rootless Linux): PODMAN_HOST_SOCK=/run/user/\$(id -u)/podman/podman.sock" >&2
      return 1
    fi
    CLAW_PODMAN_COMPOSE_ARGS+=( -f "${script_dir}/podman-compose.podman-api.yml" )
  fi
  return 0
}

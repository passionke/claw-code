# shellcheck shell=bash
# Sets CLAW_POOL_WORK_ROOT_HOST and CLAW_PODMAN_COMPOSE_ARGS. Default solve mode is podman_pool (second compose file). Author: kejiqing

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
  # Default CLAW_SOLVE_ISOLATION is podman_pool (see podman-compose + Rust); inprocess skips socket overlay.
  if [[ "${CLAW_SOLVE_ISOLATION:-podman_pool}" != "inprocess" ]]; then
    if [[ -z "${PODMAN_HOST_SOCK:-}" ]]; then
      echo "error: default mode is podman_pool; set PODMAN_HOST_SOCK to the host Podman API socket." >&2
      echo "examples: Linux rootless /run/user/\$(id -u)/podman/podman.sock · macOS: \`podman machine inspect --format '{{.ConnectionInfo.PodmanSocket.Path}}'\`" >&2
      echo "to run without a pool inside this stack: CLAW_SOLVE_ISOLATION=inprocess (no worker containers)." >&2
      return 1
    fi
    CLAW_PODMAN_COMPOSE_ARGS+=( -f "${script_dir}/podman-compose.podman-api.yml" )
  fi
  return 0
}

# shellcheck shell=bash
# Per-instance paths for CI multi-gateway on one host (shared PG). Author: kejiqing

claw_pool_rpc_root() {
  local podman_dir="${1:?podman_dir}"
  podman_dir="$(cd "${podman_dir}" && pwd)"
  local inst="${CLAW_POOL_RPC_INSTANCE:-}"
  if [[ -n "${inst}" ]]; then
    printf '%s' "${podman_dir}/.claw-pool-rpc-${inst}"
  else
    printf '%s' "${podman_dir}/.claw-pool-rpc"
  fi
}

claw_stack_workspace_bind_dir() {
  local podman_dir="${1:?podman_dir}"
  podman_dir="$(cd "${podman_dir}" && pwd)"
  local inst="${CLAW_STACK_INSTANCE:-}"
  local ws="${CLAW_POOL_WORK_ROOT_BIND_SRC:-}"

  if [[ -n "${ws}" ]]; then
    if [[ "${ws}" != /* ]]; then
      ws="${podman_dir}/${ws#./}"
    fi
    printf '%s' "${ws}"
    return 0
  fi
  if [[ -n "${inst}" ]]; then
    printf '%s' "${podman_dir}/claw-workspace-${inst}"
  elif [[ -n "${CLAW_POOL_REMOTE_BASE:-}" ]]; then
    # Remote pool: workers run elsewhere; avoid legacy root-owned claw-workspace on laptop. kejiqing
    printf '%s' "${podman_dir}/claw-workspace-remote"
  else
    printf '%s' "${podman_dir}/claw-workspace"
  fi
}

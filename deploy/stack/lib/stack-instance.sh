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

claw_strict_pool_rpc_dir() {
  printf '%s/strict' "$(claw_pool_rpc_root "${1:?podman_dir}")"
}

claw_relaxed_pool_rpc_dir() {
  printf '%s/relaxed' "$(claw_pool_rpc_root "${1:?podman_dir}")"
}

claw_stack_workspace_bind_dir() {
  local podman_dir="${1:?podman_dir}"
  podman_dir="$(cd "${podman_dir}" && pwd)"
  local inst="${CLAW_STACK_INSTANCE:-}"
  if [[ -n "${inst}" ]]; then
    printf '%s' "${podman_dir}/claw-workspace-${inst}"
  else
    printf '%s' "${podman_dir}/claw-workspace"
  fi
}

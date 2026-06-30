# shellcheck shell=bash
# Per-instance paths for CI multi-gateway on one host (shared PG). Author: kejiqing

# Gateway/OVS use compose NFS volume when NAS_BASE_URL is set (CLAW_USE_NAS_VOLUME=0 to disable). kejiqing
claw_compose_nas_volume_enabled() {
  case "${CLAW_USE_NAS_VOLUME:-}" in
    0 | false | no | off) return 1 ;;
  esac
  [[ -n "${NAS_BASE_URL:-${CLAW_E2B_NAS_SERVER:-}}" ]] || return 1
  case "${CLAW_USE_NAS_VOLUME:-auto}" in
    1 | true | yes | on | auto) return 0 ;;
    *) return 1 ;;
  esac
}

# OVS runs as e2b singleton (CLAW_OVS_BACKEND=e2b); compose openvscode-server skipped. kejiqing
claw_ovs_backend_is_e2b() {
  case "${CLAW_OVS_BACKEND:-}" in
    fc | FC) return 0 ;;
    *) return 1 ;;
  esac
}

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

  # Compose NFS volume: Gateway/OVS mount NAS inside containers; host pool uses local fallback. kejiqing
  if claw_compose_nas_volume_enabled; then
    if [[ -n "${ws}" ]]; then
      if [[ "${ws}" != /* ]]; then
        ws="${podman_dir}/${ws#./}"
      fi
      printf '%s' "${ws}"
      return 0
    fi
    if [[ -n "${inst}" ]]; then
      printf '%s' "${podman_dir}/claw-workspace-${inst}"
    else
      printf '%s' "${podman_dir}/claw-workspace"
    fi
    return 0
  fi

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

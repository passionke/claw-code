# shellcheck shell=bash
# Apply defaults from CLAW_DEPLOY_PROFILE (local | production). Author: kejiqing

# Infer profile when unset: macOS → local, else production.
claw_deploy_profile_name() {
  local p
  p="$(printf '%s' "${CLAW_DEPLOY_PROFILE:-}" | tr '[:upper:]' '[:lower:]')"
  case "${p}" in
    local | production) printf '%s' "${p}" ;;
    "")
      if [[ "$(uname -s)" == Darwin ]]; then
        printf '%s' local
      else
        printf '%s' production
      fi
      ;;
    *)
      echo "error: CLAW_DEPLOY_PROFILE must be local or production (got ${CLAW_DEPLOY_PROFILE})" >&2
      return 1
      ;;
  esac
}

# Bundled compose postgres URL when human .env omits it (same as podman-compose.yml). kejiqing
claw_default_gateway_database_url() {
  printf '%s' 'postgres://claw_gateway:clawGw9Dev_Pg@postgres:5432/claw_gateway'
}

# Set runtime/solve/tap defaults only when not already set in .env (explicit wins).
claw_apply_deploy_profile() {
  local profile
  local _profile_lib
  _profile_lib="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
  # shellcheck source=claw-region.sh
  source "${_profile_lib}/claw-region.sh"
  claw_apply_region_defaults
  # shellcheck source=release-images.sh
  source "${_profile_lib}/release-images.sh"
  profile="$(claw_deploy_profile_name)" || return 1
  export CLAW_DEPLOY_PROFILE="${profile}"

  # e2b-only: CLAW_INTERACTIVE_BACKEND / CLAW_SOLVE_ISOLATION / CLAW_OVS_BACKEND removed from .env. kejiqing

  case "${profile}" in
    local)
      unset CLAW_POOL_DAEMON_TCP CLAW_POOL_DAEMON_SOCKET CLAW_POOL_DAEMON_TCP_HOST 2>/dev/null || true
      unset CLAW_POOL_RPC_TRANSPORT 2>/dev/null || true
      if [[ "$(uname -s)" == Darwin ]]; then
        export CLAW_CONTAINER_RUNTIME="${CLAW_CONTAINER_RUNTIME:-podman}"
      else
        export CLAW_CONTAINER_RUNTIME="${CLAW_CONTAINER_RUNTIME:-docker}"
      fi
      export GATEWAY_IMAGE="${GATEWAY_IMAGE:-claw-gateway-rs:local}"
      export GATEWAY_PLAYGROUND_IMAGE="${GATEWAY_PLAYGROUND_IMAGE:-claw-gateway-playground:local}"
      export CLAW_LLM_PROXY="${CLAW_LLM_PROXY:-local}"
      export GATEWAY_HOST_PORT="${GATEWAY_HOST_PORT:-18088}"
      export GATEWAY_PLAYGROUND_HOST_PORT="${GATEWAY_PLAYGROUND_HOST_PORT:-18765}"
      export PLAYGROUND_PUBLIC_GATEWAY_BASE="${PLAYGROUND_PUBLIC_GATEWAY_BASE:-http://127.0.0.1:${GATEWAY_HOST_PORT}}"
      export CLAW_TIMEOUT_SECONDS="${CLAW_TIMEOUT_SECONDS:-900}"
      export CONTAINER_BASE_REGISTRY="${CONTAINER_BASE_REGISTRY:-docker.1ms.run}"
      # e2b observe only — no host claude-tap sidecar. Author: kejiqing
      export CLAUDE_TAP_MODE=off
      if [[ "$(uname -s)" == Darwin ]]; then
        export COMPOSE_PROJECT_NAME="${COMPOSE_PROJECT_NAME:-claw}"
        export CLAW_PODMAN_NETWORK="${CLAW_PODMAN_NETWORK:-${COMPOSE_PROJECT_NAME}_default}"
        export CLAW_WORKER_UID="$(id -u)"
        export CLAW_WORKER_GID="$(id -g)"
        export CLAW_PODMAN_BIND_MOUNT_SUFFIX=":U"
      else
        export CLAW_PODMAN_NETWORK="${CLAW_PODMAN_NETWORK:-stack_default}"
      fi
      ;;
    production)
      export CLAW_CONTAINER_RUNTIME="${CLAW_CONTAINER_RUNTIME:-docker}"
      export CLAW_LLM_PROXY="${CLAW_LLM_PROXY:-direct}"
      export CLAW_IMAGE_REGISTRY="${CLAW_IMAGE_REGISTRY:-acr}"
      export GATEWAY_HOST_PORT="${GATEWAY_HOST_PORT:-8088}"
      export GATEWAY_PLAYGROUND_HOST_PORT="${GATEWAY_PLAYGROUND_HOST_PORT:-18765}"
      export CLAW_GATEWAY_PG_IMAGE="${CLAW_GATEWAY_PG_IMAGE:-docker.io/library/postgres:17-alpine}"
      # e2b observe singleton; never host claude-tap. Author: kejiqing
      export CLAUDE_TAP_MODE=off
      ;;
  esac

  if [[ "${CLAW_USE_DOCKER:-0}" == "1" && "${CLAW_CONTAINER_RUNTIME:-}" == "auto" ]]; then
    export CLAW_CONTAINER_RUNTIME=docker
  fi

  # Local docker compose network defaults (gateway/playground only; tap is e2b). Author: kejiqing
  if [[ "${profile}" == local && "${CLAW_CONTAINER_RUNTIME:-}" == docker ]]; then
    export COMPOSE_PROJECT_NAME="${COMPOSE_PROJECT_NAME:-claw}"
    export CLAW_DOCKER_NETWORK="${CLAW_DOCKER_NETWORK:-${COMPOSE_PROJECT_NAME}_default}"
  fi

  export CLAW_GATEWAY_DATABASE_URL="${CLAW_GATEWAY_DATABASE_URL:-$(claw_default_gateway_database_url)}"
  export CLAW_GATEWAY_PG_HOST_PORT="${CLAW_GATEWAY_PG_HOST_PORT:-5433}"
  if [[ "${profile}" == local ]]; then
    export CLAW_CLUSTER_ID="${CLAW_CLUSTER_ID:-local-dev}"
  fi

  return 0
}

# Reject removed backend/tap knobs still present in human .env. kejiqing
claw_reject_removed_backend_env() {
  local v raw
  for v in CLAW_INTERACTIVE_BACKEND CLAW_OVS_BACKEND CLAW_SOLVE_ISOLATION; do
    raw="${!v:-}"
    [[ -z "${raw}" ]] && continue
    echo "error: ${v} is removed from .env (e2b-only); delete this line" >&2
    return 1
  done
  # apply always sets CLAUDE_TAP_MODE=off; reject leftover human .env (e2b observe only). Author: kejiqing
  if [[ -n "${CLAUDE_TAP_MODE:-}" && "${CLAUDE_TAP_MODE}" != off ]]; then
    echo "error: CLAUDE_TAP_MODE is removed (e2b observe only); delete this line from .env" >&2
    return 1
  fi
  return 0
}

# Fail fast on common deploy mistakes.
claw_validate_deploy_profile() {
  local profile rt
  profile="$(claw_deploy_profile_name)" || return 1
  rt="$(claw_container_runtime_cli 2>/dev/null || true)"

  claw_reject_removed_backend_env || return 1

  if [[ -n "${PODMAN_HOST_SOCK:-}" ]]; then
    echo "error: PODMAN_HOST_SOCK is removed; delete it from .env" >&2
    return 1
  fi

  case "${profile}" in
    local) ;;
    production)
      case "${CLAW_LLM_PROXY:-direct}" in
        local)
          echo "error: CLAW_DEPLOY_PROFILE=production does not use CLAW_LLM_PROXY=local (sidecar tap)" >&2
          echo "hint: use direct (default) or remote + CLAW_TAP_PROXY_URL for a shared tap service" >&2
          return 1
          ;;
        remote)
          if [[ -z "${CLAW_TAP_PROXY_URL:-}" ]]; then
            echo "error: CLAW_LLM_PROXY=remote requires CLAW_TAP_PROXY_URL (shared claude-tap base URL)" >&2
            return 1
          fi
          ;;
        direct) ;;
        *)
          echo "error: CLAW_LLM_PROXY must be direct, remote, or local (got ${CLAW_LLM_PROXY})" >&2
          return 1
          ;;
      esac
      if [[ "${rt}" == podman ]]; then
        echo "error: CLAW_DEPLOY_PROFILE=production expects CLAW_CONTAINER_RUNTIME=docker" >&2
        return 1
      fi
      if [[ -z "${GATEWAY_IMAGE:-}" && -z "${CLAW_IMAGE_RELEASE_TAG:-}" ]]; then
        echo "error: production needs GATEWAY_IMAGE, --release release-vX.Y.Z, or deploy/stack/.claw-image-release.env" >&2
        return 1
      fi
      ;;
  esac

  return 0
}

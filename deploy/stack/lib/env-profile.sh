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
  # shellcheck source=release-images.sh
  source "${_profile_lib}/release-images.sh"
  profile="$(claw_deploy_profile_name)" || return 1
  export CLAW_DEPLOY_PROFILE="${profile}"

  # e2b is the only worker execution backend. kejiqing
  export CLAW_INTERACTIVE_BACKEND="${CLAW_INTERACTIVE_BACKEND:-e2b}"
  export CLAW_OVS_BACKEND="${CLAW_OVS_BACKEND:-e2b}"
  export CLAW_SOLVE_ISOLATION="${CLAW_SOLVE_ISOLATION:-e2b}"

  case "${profile}" in
    local)
      unset CLAW_POOL_DAEMON_TCP CLAW_POOL_DAEMON_SOCKET CLAW_POOL_DAEMON_TCP_HOST 2>/dev/null || true
      unset CLAW_POOL_RPC_TRANSPORT 2>/dev/null || true
      export CLAW_CONTAINER_RUNTIME="${CLAW_CONTAINER_RUNTIME:-podman}"
      export GATEWAY_IMAGE="${GATEWAY_IMAGE:-claw-gateway-rs:local}"
      export GATEWAY_PLAYGROUND_IMAGE="${GATEWAY_PLAYGROUND_IMAGE:-claw-gateway-playground:local}"
      # shellcheck source=/dev/null
      [[ -f "${CLAW_REPO_ROOT:-}/deploy/stack/ovs-image.env" ]] && source "${CLAW_REPO_ROOT}/deploy/stack/ovs-image.env"
      if [[ -z "${CLAW_OVS_UPSTREAM_IMAGE:-}" && -f "$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)/ovs-image.env" ]]; then
        # shellcheck source=/dev/null
        source "$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)/ovs-image.env"
      fi
      export CLAW_OVS_UPSTREAM_IMAGE="${CLAW_OVS_UPSTREAM_IMAGE:-crpi-cf9vxpq3n8or17mw.cn-hangzhou.personal.cr.aliyuncs.com/passionke/openvscode-server:1.109.5-ovs-chat}"
      export CLAW_OVS_IMAGE="${CLAW_OVS_IMAGE:-${CLAW_OVS_UPSTREAM_IMAGE}}"
      export CLAW_LLM_PROXY="${CLAW_LLM_PROXY:-local}"
      export GATEWAY_HOST_PORT="${GATEWAY_HOST_PORT:-18088}"
      export GATEWAY_PLAYGROUND_HOST_PORT="${GATEWAY_PLAYGROUND_HOST_PORT:-18765}"
      export PLAYGROUND_PUBLIC_GATEWAY_BASE="${PLAYGROUND_PUBLIC_GATEWAY_BASE:-http://127.0.0.1:${GATEWAY_HOST_PORT}}"
      export CLAW_TIMEOUT_SECONDS="${CLAW_TIMEOUT_SECONDS:-900}"
      export CONTAINER_BASE_REGISTRY="${CONTAINER_BASE_REGISTRY:-docker.1ms.run}"
      # macOS Podman: one network for gateway + docker tap. e2b workers run on e2b. kejiqing
      if [[ "$(uname -s)" == Darwin ]]; then
        export COMPOSE_PROJECT_NAME="${COMPOSE_PROJECT_NAME:-claw}"
        export CLAUDE_TAP_MODE="${CLAUDE_TAP_MODE:-docker}"
        export CLAUDE_TAP_DOCKER_NETWORK="${CLAUDE_TAP_DOCKER_NETWORK:-${COMPOSE_PROJECT_NAME}_default}"
        export CLAW_PODMAN_NETWORK="${CLAW_PODMAN_NETWORK:-${COMPOSE_PROJECT_NAME}_default}"
        export CLAW_WORKER_UID="$(id -u)"
        export CLAW_WORKER_GID="$(id -g)"
        export CLAW_PODMAN_BIND_MOUNT_SUFFIX=":U"
      else
        export CLAUDE_TAP_MODE="${CLAUDE_TAP_MODE:-native}"
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
      export CLAUDE_TAP_IMAGE="${CLAUDE_TAP_IMAGE:-$(claw_default_claude_tap_image)}"
      if [[ -z "${CLAW_OVS_UPSTREAM_IMAGE:-}" && -f "$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)/ovs-image.env" ]]; then
        # shellcheck source=/dev/null
        source "$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)/ovs-image.env"
      fi
      export CLAW_OVS_UPSTREAM_IMAGE="${CLAW_OVS_UPSTREAM_IMAGE:-crpi-cf9vxpq3n8or17mw.cn-hangzhou.personal.cr.aliyuncs.com/passionke/openvscode-server:1.109.5-ovs-chat}"
      export CLAW_OVS_IMAGE="${CLAW_OVS_IMAGE:-${CLAW_OVS_UPSTREAM_IMAGE}}"
      ;;
  esac

  if [[ "${CLAW_USE_DOCKER:-0}" == "1" && "${CLAW_CONTAINER_RUNTIME:-}" == "auto" ]]; then
    export CLAW_CONTAINER_RUNTIME=docker
  fi

  # Local docker: claude-tap sidecar only (solve/interactive on e2b). kejiqing
  if [[ "${profile}" == local && "${CLAW_CONTAINER_RUNTIME:-}" == docker ]]; then
    export CLAUDE_TAP_MODE="${CLAUDE_TAP_MODE:-docker}"
    export CLAUDE_TAP_IMAGE="${CLAUDE_TAP_IMAGE:-$(claw_default_claude_tap_image)}"
    export COMPOSE_PROJECT_NAME="${COMPOSE_PROJECT_NAME:-claw}"
    export CLAUDE_TAP_DOCKER_NETWORK="${CLAUDE_TAP_DOCKER_NETWORK:-${COMPOSE_PROJECT_NAME}_default}"
    export CLAW_DOCKER_NETWORK="${CLAW_DOCKER_NETWORK:-${COMPOSE_PROJECT_NAME}_default}"
    export CLAUDE_TAP_PUBLISH_PROXY="${CLAUDE_TAP_PUBLISH_PROXY:-127.0.0.1:8080:8080}"
    export CLAUDE_TAP_PUBLISH_LIVE="${CLAUDE_TAP_PUBLISH_LIVE:-0.0.0.0:3000:3000}"
  fi

  export CLAW_GATEWAY_DATABASE_URL="${CLAW_GATEWAY_DATABASE_URL:-$(claw_default_gateway_database_url)}"
  export CLAW_GATEWAY_PG_HOST_PORT="${CLAW_GATEWAY_PG_HOST_PORT:-5433}"
  if [[ "${profile}" == local ]]; then
    export CLAW_CLUSTER_ID="${CLAW_CLUSTER_ID:-local-dev}"
  fi

  return 0
}

# Fail fast on common deploy mistakes.
claw_validate_deploy_profile() {
  local profile rt iso
  profile="$(claw_deploy_profile_name)" || return 1
  rt="$(claw_container_runtime_cli 2>/dev/null || true)"
  iso="${CLAW_SOLVE_ISOLATION:-e2b}"

  if [[ -n "${PODMAN_HOST_SOCK:-}" ]]; then
    echo "error: PODMAN_HOST_SOCK is removed; delete it from .env" >&2
    return 1
  fi

  if [[ "${iso}" != e2b ]]; then
    echo "error: CLAW_SOLVE_ISOLATION must be e2b (got ${iso}); podman_pool/docker_pool removed" >&2
    return 1
  fi

  case "${CLAW_INTERACTIVE_BACKEND:-e2b}" in
    e2b) ;;
    *)
      echo "error: CLAW_INTERACTIVE_BACKEND must be e2b (got ${CLAW_INTERACTIVE_BACKEND})" >&2
      return 1
      ;;
  esac

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

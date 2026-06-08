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

  case "${profile}" in
    local)
      # One pool URL: HTTP on 9944 (live SSE + POST /v1/pool/rpc). No 9943 TCP / unix. kejiqing
      if [[ "${CLAW_CONTAINER_RUNTIME:-podman}" == docker || "${CLAW_USE_DOCKER:-0}" == "1" ]]; then
        export CLAW_POOL_HTTP_BASE="${CLAW_POOL_HTTP_BASE:-http://host.docker.internal:9944}"
      else
        export CLAW_POOL_HTTP_BASE="${CLAW_POOL_HTTP_BASE:-http://host.containers.internal:9944}"
      fi
      unset CLAW_POOL_DAEMON_TCP CLAW_POOL_DAEMON_SOCKET CLAW_POOL_DAEMON_TCP_HOST 2>/dev/null || true
      unset CLAW_POOL_RPC_TRANSPORT 2>/dev/null || true
      export CLAW_CONTAINER_RUNTIME="${CLAW_CONTAINER_RUNTIME:-podman}"
      export CLAW_SOLVE_ISOLATION="${CLAW_SOLVE_ISOLATION:-podman_pool}"
      export GATEWAY_IMAGE="${GATEWAY_IMAGE:-claw-gateway-rs:local}"
      export GATEWAY_PLAYGROUND_IMAGE="${GATEWAY_PLAYGROUND_IMAGE:-claw-gateway-playground:local}"
      export CLAW_PODMAN_IMAGE="${CLAW_PODMAN_IMAGE:-claw-gateway-worker:local}"
      export CLAW_RELAXED_PODMAN_IMAGE="${CLAW_RELAXED_PODMAN_IMAGE:-claw-gateway-worker-relaxed:local}"
      export CLAW_LLM_PROXY="${CLAW_LLM_PROXY:-local}"
      export CLAUDE_TAP_MODE="${CLAUDE_TAP_MODE:-native}"
      export GATEWAY_HOST_PORT="${GATEWAY_HOST_PORT:-18088}"
      export GATEWAY_PLAYGROUND_HOST_PORT="${GATEWAY_PLAYGROUND_HOST_PORT:-18765}"
      export PLAYGROUND_PUBLIC_GATEWAY_BASE="${PLAYGROUND_PUBLIC_GATEWAY_BASE:-http://127.0.0.1:${GATEWAY_HOST_PORT}}"
      export CONTAINER_BASE_REGISTRY="${CONTAINER_BASE_REGISTRY:-docker.1ms.run}"
      export CLAW_PODMAN_NETWORK="${CLAW_PODMAN_NETWORK:-stack_default}"
      ;;
    production)
      export CLAW_CONTAINER_RUNTIME="${CLAW_CONTAINER_RUNTIME:-docker}"
      export CLAW_SOLVE_ISOLATION="${CLAW_SOLVE_ISOLATION:-docker_pool}"
      export CLAW_POOL_HOST_DAEMON="${CLAW_POOL_HOST_DAEMON:-1}"
      export CLAW_POOL_DAEMON_SKIP_BUILD="${CLAW_POOL_DAEMON_SKIP_BUILD:-1}"
      # Same port for live + RPC unless overridden in .env. kejiqing
      if [[ -z "${CLAW_POOL_HTTP_BASE:-}" && -n "${CLAW_POOL_ADVERTISE_HOST:-}" ]]; then
        export CLAW_POOL_HTTP_BASE="http://${CLAW_POOL_ADVERTISE_HOST}:${CLAW_POOL_HTTP_PORT:-9944}"
      fi
      export CLAW_LLM_PROXY="${CLAW_LLM_PROXY:-direct}"
      export CLAW_IMAGE_REGISTRY="${CLAW_IMAGE_REGISTRY:-acr}"
      export GATEWAY_HOST_PORT="${GATEWAY_HOST_PORT:-8088}"
      export GATEWAY_PLAYGROUND_HOST_PORT="${GATEWAY_PLAYGROUND_HOST_PORT:-18765}"
      export CLAW_GATEWAY_PG_IMAGE="${CLAW_GATEWAY_PG_IMAGE:-docker.io/library/postgres:17-alpine}"
      export CLAUDE_TAP_IMAGE="${CLAUDE_TAP_IMAGE:-$(claw_default_claude_tap_image)}"
      ;;
  esac

  if [[ "${CLAW_USE_DOCKER:-0}" == "1" && "${CLAW_CONTAINER_RUNTIME:-}" == "auto" ]]; then
    export CLAW_CONTAINER_RUNTIME=docker
  fi

  # Linux docker + local pack-deploy: tap/solve defaults live in profile, not human .env. kejiqing
  if [[ "${profile}" == local && "${CLAW_CONTAINER_RUNTIME:-}" == docker ]]; then
    export CLAW_SOLVE_ISOLATION="${CLAW_SOLVE_ISOLATION:-docker_pool}"
    export CLAUDE_TAP_MODE="${CLAUDE_TAP_MODE:-docker}"
    export CLAUDE_TAP_IMAGE="${CLAUDE_TAP_IMAGE:-$(claw_default_claude_tap_image)}"
    export COMPOSE_PROJECT_NAME="${COMPOSE_PROJECT_NAME:-claw}"
    export CLAUDE_TAP_DOCKER_NETWORK="${CLAUDE_TAP_DOCKER_NETWORK:-${COMPOSE_PROJECT_NAME}_default}"
    export CLAW_DOCKER_NETWORK="${CLAW_DOCKER_NETWORK:-${COMPOSE_PROJECT_NAME}_default}"
    # Local dev: publish Live (3000) + proxy (8080) on host for trace viewer / curl debug.
    # Local docker: publish tap on loopback; Admin host defaults to 127.0.0.1 unless CLAW_POOL_ADVERTISE_HOST set.
    export CLAUDE_TAP_PUBLISH_PROXY="${CLAUDE_TAP_PUBLISH_PROXY:-127.0.0.1:8080:8080}"
    export CLAUDE_TAP_PUBLISH_LIVE="${CLAUDE_TAP_PUBLISH_LIVE:-0.0.0.0:3000:3000}"
  fi

  export CLAW_GATEWAY_DATABASE_URL="${CLAW_GATEWAY_DATABASE_URL:-$(claw_default_gateway_database_url)}"
  export CLAW_GATEWAY_PG_HOST_PORT="${CLAW_GATEWAY_PG_HOST_PORT:-5433}"
  if [[ "${profile}" == local ]]; then
    export CLAW_CLUSTER_ID="${CLAW_CLUSTER_ID:-local-dev}"
  fi

  claw_sync_solve_worker_image_prefix || return 1
  return 0
}

# When solve mode is docker_pool, drop stale CLAW_PODMAN_IMAGE from human .env unless explicit opt-out.
claw_sync_solve_worker_image_prefix() {
  case "${CLAW_SOLVE_ISOLATION:-podman_pool}" in
    docker_pool)
      if [[ "${CLAW_POOL_WORKER_IMAGE_EXPLICIT:-0}" != "1" && -n "${CLAW_PODMAN_IMAGE:-}" && -z "${CLAW_DOCKER_IMAGE:-}" ]]; then
        export CLAW_DOCKER_IMAGE="${CLAW_DOCKER_IMAGE:-${CLAW_PODMAN_IMAGE}}"
        unset CLAW_PODMAN_IMAGE
      fi
      ;;
    podman_pool)
      if [[ "${CLAW_POOL_WORKER_IMAGE_EXPLICIT:-0}" != "1" && -n "${CLAW_DOCKER_IMAGE:-}" && -z "${CLAW_PODMAN_IMAGE:-}" ]]; then
        export CLAW_PODMAN_IMAGE="${CLAW_PODMAN_IMAGE:-${CLAW_DOCKER_IMAGE}}"
        unset CLAW_DOCKER_IMAGE
      fi
      ;;
  esac
  return 0
}

# Fail fast on common 1.4.x deploy mistakes (socket / pool TCP / mixed runtimes).
claw_validate_deploy_profile() {
  local profile rt iso
  profile="$(claw_deploy_profile_name)" || return 1
  rt="$(claw_container_runtime_cli 2>/dev/null || true)"
  iso="${CLAW_SOLVE_ISOLATION:-podman_pool}"

  if [[ -n "${PODMAN_HOST_SOCK:-}" ]]; then
    echo "error: PODMAN_HOST_SOCK is removed; delete it from .env" >&2
    return 1
  fi

  case "${profile}" in
    local)
      if [[ "${iso}" == docker_pool && "${rt}" == podman ]]; then
        echo "error: CLAW_DEPLOY_PROFILE=local expects podman_pool, not docker_pool" >&2
        echo "hint: remove CLAW_SOLVE_ISOLATION=docker_pool from .env or set CLAW_DEPLOY_PROFILE=production" >&2
        return 1
      fi
      ;;
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
      if [[ "${iso}" == podman_pool ]]; then
        echo "error: CLAW_DEPLOY_PROFILE=production expects docker_pool (got podman_pool)" >&2
        return 1
      fi
      if [[ "${rt}" == podman ]]; then
        echo "error: CLAW_DEPLOY_PROFILE=production expects CLAW_CONTAINER_RUNTIME=docker" >&2
        return 1
      fi
      if [[ -z "${GATEWAY_IMAGE:-}" && -z "${CLAW_IMAGE_RELEASE_TAG:-}" ]]; then
        echo "error: production needs GATEWAY_IMAGE, --release release-vX.Y.Z, or deploy/stack/.claw-image-release.env" >&2
        return 1
      fi
      if ! claw_pool_daemon_on_host; then
        echo "error: Linux production uses host claw-pool-daemon (unset CLAW_POOL_HOST_DAEMON=0)" >&2
        return 1
      fi
      ;;
  esac

  if [[ "${iso}" == docker_pool && -z "${CLAW_DOCKER_IMAGE:-}" ]]; then
    echo "error: CLAW_DOCKER_IMAGE unset for docker_pool (set GATEWAY_IMAGE + --release, or CLAW_IMAGE_PREFIX)" >&2
    return 1
  fi
  if [[ "${iso}" == podman_pool && -z "${CLAW_PODMAN_IMAGE:-}" ]]; then
    echo "error: CLAW_PODMAN_IMAGE unset for podman_pool" >&2
    return 1
  fi

  return 0
}

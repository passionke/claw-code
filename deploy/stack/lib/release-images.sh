# shellcheck shell=bash
# Helpers for pinning remote registry gateway/worker tags (GHCR, ACR, …) without editing .env. Author: kejiqing

# Default namespaces when `.env` omits CLAW_IMAGE_PREFIX (personal ACR + passionke org on GHCR).
claw_default_acr_image_prefix() {
  printf '%s' "${CLAW_ACR_IMAGE_PREFIX:-crpi-cf9vxpq3n8or17mw.cn-hangzhou.personal.cr.aliyuncs.com/passionke}"
}

claw_default_ghcr_image_prefix() {
  printf '%s' "${CLAW_GHCR_DEFAULT_PREFIX:-ghcr.io/passionke}"
}

claw_image_registry_prefix_from_env() {
  # CLAW_IMAGE_PREFIX wins (registry-agnostic name); CLAW_GHCR_PREFIX kept for back-compat.
  local prefix="${CLAW_IMAGE_PREFIX:-${CLAW_GHCR_PREFIX:-}}"
  if [[ -n "$prefix" ]]; then
    printf '%s' "$prefix"
    return 0
  fi
  local gw="${GATEWAY_IMAGE:-}"
  if [[ "$gw" == *"/claw-code:"* ]]; then
    printf '%s' "${gw%%/claw-code:*}"
    return 0
  fi
  if [[ "$gw" == */claw-code ]]; then
    printf '%s' "${gw%/claw-code}"
    return 0
  fi

  # No explicit prefix and no claw-code in GATEWAY_IMAGE (e.g. local :local tags): pick backend.
  local backend="${CLAW_IMAGE_REGISTRY:-acr}"
  backend="$(printf '%s' "$backend" | tr '[:upper:]' '[:lower:]')"
  case "$backend" in
    ghcr)
      claw_default_ghcr_image_prefix
      return 0
      ;;
    acr | *)
      claw_default_acr_image_prefix
      return 0
      ;;
  esac
}

# After sourcing .env: set GATEWAY_IMAGE + worker image to <prefix>/...:<tag>.
claw_apply_release_image_tag() {
  local tag="${1:?}"
  local prefix
  prefix="$(claw_image_registry_prefix_from_env)"
  export GATEWAY_IMAGE="${prefix}/claw-code:${tag}"
  case "${CLAW_SOLVE_ISOLATION:-podman_pool}" in
    docker_pool) export CLAW_DOCKER_IMAGE="${prefix}/claw-gateway-worker:${tag}" ;;
    *) export CLAW_PODMAN_IMAGE="${prefix}/claw-gateway-worker:${tag}" ;;
  esac
}

# One upgrade knob: pool worker image follows GATEWAY_IMAGE tag/registry (unless explicit opt-out). kejiqing
claw_export_pool_worker_image_matched_to_gateway() {
  local gw="${GATEWAY_IMAGE:-}"
  [[ -n "$gw" ]] || return 0
  [[ "$gw" == *claw-code* ]] || return 0
  if [[ "${CLAW_POOL_WORKER_IMAGE_EXPLICIT:-0}" == "1" ]]; then
    return 0
  fi
  local derived="${gw/claw-code/claw-gateway-worker}"
  case "${CLAW_SOLVE_ISOLATION:-podman_pool}" in
    docker_pool) export CLAW_DOCKER_IMAGE="$derived" ;;
    podman_pool) export CLAW_PODMAN_IMAGE="$derived" ;;
  esac
}

# Compose pool sidecar reads env files from disk — last file wins; override stale CLAW_*_IMAGE in repo .env.
claw_write_pool_worker_env_override() {
  local script_dir="${1:?}"
  local f="${script_dir}/.claw-pool-worker.env"
  local gw="${GATEWAY_IMAGE:-}"
  if [[ "$gw" != *claw-code* ]] || [[ "${CLAW_POOL_WORKER_IMAGE_EXPLICIT:-0}" == "1" ]]; then
    rm -f "${f}"
    return 0
  fi
  case "${CLAW_SOLVE_ISOLATION:-podman_pool}" in
    docker_pool)
      [[ -n "${CLAW_DOCKER_IMAGE:-}" ]] || {
        rm -f "${f}"
        return 0
      }
      {
        printf '%s\n' '# GENERATED — do not edit. CLAW_DOCKER_IMAGE synced from GATEWAY_IMAGE (claw-code→claw-gateway-worker). Set CLAW_POOL_WORKER_IMAGE_EXPLICIT=1 to use repo .env only. kejiqing'
        printf '%s\n' "CLAW_DOCKER_IMAGE=${CLAW_DOCKER_IMAGE}"
      } >"${f}"
      ;;
    *)
      [[ -n "${CLAW_PODMAN_IMAGE:-}" ]] || {
        rm -f "${f}"
        return 0
      }
      {
        printf '%s\n' '# GENERATED — do not edit. CLAW_PODMAN_IMAGE synced from GATEWAY_IMAGE. kejiqing'
        printf '%s\n' "CLAW_PODMAN_IMAGE=${CLAW_PODMAN_IMAGE}"
      } >"${f}"
      ;;
  esac
}

# Compose reads --env-file from disk; second file overrides keys from repo .env.
claw_write_release_pin_env() {
  local podman_dir="$1"
  claw_export_pool_worker_image_matched_to_gateway
  local f="${podman_dir}/.claw-image-release.env"
  {
    printf '%s\n' "# GENERATED — do not edit. rm file to drop pin. Author: kejiqing"
    printf '%s\n' "GATEWAY_IMAGE=${GATEWAY_IMAGE}"
    case "${CLAW_SOLVE_ISOLATION:-podman_pool}" in
      docker_pool) printf '%s\n' "CLAW_DOCKER_IMAGE=${CLAW_DOCKER_IMAGE}" ;;
      *) printf '%s\n' "CLAW_PODMAN_IMAGE=${CLAW_PODMAN_IMAGE}" ;;
    esac
  } >"${f}"
}

claw_parse_up_release_args() {
  CLAW_IMAGE_RELEASE_TAG=""
  while [[ $# -gt 0 ]]; do
    case "$1" in
      --release=*)
        CLAW_IMAGE_RELEASE_TAG="${1#*=}"
        shift
        ;;
      --release)
        if [[ $# -lt 2 ]]; then
          echo "error: --release requires a value (e.g. release-v1.0.22)" >&2
          return 1
        fi
        CLAW_IMAGE_RELEASE_TAG="$2"
        shift 2
        ;;
      -h | --help)
        echo "usage: $0 [--release <tag>|release-v*]" >&2
        echo "  --release <tag>   pin gateway + worker images for this run; writes deploy/stack/.claw-image-release.env" >&2
        echo "                    (merged after .env). Uses CLAW_IMAGE_PREFIX if set; else CLAW_IMAGE_REGISTRY=acr (default ACR) or ghcr." >&2
        echo "  release-v*        same as --release release-v*" >&2
        echo "  Subsequent runs without --release still use .claw-image-release.env if present; remove that file to follow .env only." >&2
        return 2
        ;;
      release-v*)
        CLAW_IMAGE_RELEASE_TAG="$1"
        shift
        ;;
      *)
        echo "error: unknown argument: $1 (try --help)" >&2
        return 1
        ;;
    esac
  done
}

claw_compose_with_root_env() {
  local podman_dir="$1"
  local repo_env="$2"
  shift 2
  local sticky="${podman_dir}/.claw-image-release.env"
  if [[ -f "${sticky}" ]]; then
    # Docker Compose prefers process environment over `--env-file`. `up.sh` sources `.env` before compose,
    # which would pin old GATEWAY_IMAGE and defeat the sticky override — drop these for this invocation.
    (
      unset GATEWAY_IMAGE CLAW_DOCKER_IMAGE CLAW_PODMAN_IMAGE || true
      claw_compose --env-file "${repo_env}" --env-file "${sticky}" "$@"
    )
  else
    claw_compose --env-file "${repo_env}" "$@"
  fi
}

# shellcheck shell=bash
# Helpers for pinning Gateway/Admin image tags without editing .env. Author: kejiqing

# Default namespaces when `.env` omits CLAW_IMAGE_PREFIX (personal ACR + passionke org on GHCR).
claw_default_acr_image_prefix() {
  printf '%s' "${CLAW_ACR_IMAGE_PREFIX:-crpi-cf9vxpq3n8or17mw.cn-hangzhou.personal.cr.aliyuncs.com/passionke}"
}

claw_default_ghcr_image_prefix() {
  printf '%s' "${CLAW_GHCR_DEFAULT_PREFIX:-ghcr.io/passionke}"
}

claw_default_claude_tap_image() {
  printf '%s/claw-tap:latest' "$(claw_default_acr_image_prefix)"
}

claw_image_registry_prefix_from_env() {
  # CLAW_IMAGE_PREFIX wins (registry-agnostic name); CLAW_GHCR_PREFIX kept for back-compat.
  local prefix="${CLAW_IMAGE_PREFIX:-${CLAW_GHCR_PREFIX:-}}"
  if [[ -n "$prefix" ]]; then
    printf '%s' "$prefix"
    return 0
  fi
  local gw="${GATEWAY_IMAGE:-}"
  # Prefer new name http-gateway-rs; accept legacy claw-code for prefix parse only.
  for name in http-gateway-rs claw-code; do
    if [[ "$gw" == *"/${name}:"* ]]; then
      printf '%s' "${gw%%/"${name}":*}"
      return 0
    fi
    if [[ "$gw" == */"${name}" ]]; then
      printf '%s' "${gw%/"${name}"}"
      return 0
    fi
  done

  # No explicit prefix (e.g. local :local tags): pick backend.
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

# After sourcing .env: set only Gateway/Admin images to <prefix>/...:<tag>.
# Nora / ship-release publish names: claw-code + claw-gateway-playground.
# Author: kejiqing
#   CLAW_RELEASE_PLAYGROUND_IMAGE  — pin playground (e.g. claw-gateway-playground:local)
claw_apply_release_image_tag() {
  local tag="${1:?}"
  local prefix
  prefix="$(claw_image_registry_prefix_from_env)"
  export GATEWAY_IMAGE="${prefix}/claw-code:${tag}"
  if [[ -n "${CLAW_RELEASE_PLAYGROUND_IMAGE:-}" ]]; then
    export GATEWAY_PLAYGROUND_IMAGE="${CLAW_RELEASE_PLAYGROUND_IMAGE}"
  else
    export GATEWAY_PLAYGROUND_IMAGE="${prefix}/claw-gateway-playground:${tag}"
  fi
}

# Protocol images are explicit and never derived from a Gateway release tag. Author: kejiqing
claw_resolve_worker_image_ref() {
  local image="${CLAW_E2B_WORKER_IMAGE:-${CLAW_PODMAN_IMAGE:-${CLAW_DOCKER_IMAGE:-}}}"
  if [[ -z "$image" ]]; then
    echo "set CLAW_E2B_WORKER_IMAGE to an independently published protocol image" >&2
    return 1
  fi
  printf '%s' "$image"
}

# Local Gateway/Admin pack-deploy. Author: kejiqing
claw_apply_pack_deploy_image_tag() {
  local tag="${1:?pack-deploy image tag required}"
  export GATEWAY_IMAGE="claw-gateway-rs:${tag}"
  export GATEWAY_PLAYGROUND_IMAGE="claw-gateway-playground:${tag}"
}

# e2b owns Worker lifecycle; Gateway release does not write Worker image overrides.
claw_write_pool_worker_env_override() {
  local script_dir="${1:?}"
  local f="${script_dir}/.claw-pool-worker.env"
  printf '%s\n' '# GENERATED — Gateway releases do not pin e2b Worker protocol images. kejiqing' >"${f}"
}

# Compose reads --env-file from disk; second file overrides keys from repo .env.
claw_write_release_pin_env() {
  local podman_dir="$1"
  local f="${podman_dir}/.claw-image-release.env"
  {
    printf '%s\n' "# GENERATED — do not edit. rm file to drop pin. Author: kejiqing"
    printf '%s\n' "GATEWAY_IMAGE=${GATEWAY_IMAGE}"
    printf '%s\n' "GATEWAY_PLAYGROUND_IMAGE=${GATEWAY_PLAYGROUND_IMAGE}"
  } >"${f}"
}

# Skip remote pull when CI built images on this host (CLAW_RELEASE_SKIP_PULL=1 + image exists). kejiqing
# Short local names (no `/`, e.g. claw-gateway-playground:local) must not hit docker.io. Author: kejiqing
claw_release_pull_image_if_needed() {
  local rt="$1"
  local image="$2"
  if "${rt}" image inspect "${image}" >/dev/null 2>&1; then
    case "${image}" in
      */*) ;;
      *)
        echo "skip pull ${image} (local short name present)" >&2
        return 0
        ;;
    esac
    if [[ "${CLAW_RELEASE_SKIP_PULL:-0}" == "1" ]]; then
      echo "skip pull ${image} (CLAW_RELEASE_SKIP_PULL=1, local image present)" >&2
      return 0
    fi
  fi
  echo "pull ${image} …" >&2
  "${rt}" pull "${image}"
}

# After sourcing repo .env (may contain :local image tags). Prefer --release tag, then sticky pin file.
# Writes deploy/stack/.claw-image-release.env + .claw-pool-worker.env when a release tag is active.
claw_reapply_pool_image_pins() {
  local podman_dir="${1:?}"
  if [[ -n "${CLAW_IMAGE_RELEASE_TAG:-}" ]]; then
    claw_apply_release_image_tag "${CLAW_IMAGE_RELEASE_TAG}"
    claw_write_release_pin_env "${podman_dir}"
  elif [[ -f "${podman_dir}/.claw-image-release.env" ]]; then
    set -a
    # shellcheck disable=SC1090,SC1091
    source "${podman_dir}/.claw-image-release.env"
    set +a
    claw_write_release_pin_env "${podman_dir}"
  fi
  claw_write_pool_worker_env_override "${podman_dir}"
  if [[ -f "${podman_dir}/.claw-pool-worker.env" ]]; then
    set -a
    # shellcheck disable=SC1090,SC1091
    source "${podman_dir}/.claw-pool-worker.env"
    set +a
  fi
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
        echo "  --release <tag>   pin gateway + worker to same tag (CLAW_DOCKER_IMAGE follows; writes .claw-image-release.env)" >&2
        echo "                    Uses CLAW_IMAGE_PREFIX if set; else CLAW_IMAGE_REGISTRY=acr (default ACR) or ghcr." >&2
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
  local profile_sh="${podman_dir}/lib/env-profile.sh"
  # Legacy `docker-compose` v1 (common behind podman on Aliyun) accepts only one `--env-file`;
  # a second `--env-file` prints usage and exits. Source env in a subshell instead. Author: kejiqing
  (
    unset GATEWAY_IMAGE GATEWAY_PLAYGROUND_IMAGE CLAW_DOCKER_IMAGE CLAW_PODMAN_IMAGE || true
    set -a
    # shellcheck disable=SC1090
    source "${repo_env}"
    if [[ -f "${sticky}" ]]; then
      # shellcheck disable=SC1090
      source "${sticky}"
    fi
    # Apply profile defaults (GATEWAY_IMAGE=claw-gateway-rs:local, etc.) for compose interpolation.
    if [[ -f "${profile_sh}" ]]; then
      # shellcheck disable=SC1090
      source "${profile_sh}"
      claw_apply_deploy_profile
    fi
    set +a
    claw_compose "$@"
  )
}

# shellcheck shell=bash
# Region switch: region=china → CN mirrors (apt/cargo/rustup/registry/e2b templates). Author: kejiqing
#
# Machine-level: export region=china in ~/.bashrc (non-interactive deploy reads it too).
# Optional override in repo .env: REGION=china | global

claw_region_load() {
  [[ -n "${region:-}${REGION:-}${CLAW_REGION:-}" ]] && return 0
  if [[ -f "${HOME}/.claw-region" ]]; then
    set -a
    # shellcheck source=/dev/null
    source "${HOME}/.claw-region"
    set +a
    return 0
  fi
  local line
  if [[ -f "${HOME}/.bashrc" ]]; then
    line="$(grep -E '^[[:space:]]*(export[[:space:]]+)?region=' "${HOME}/.bashrc" 2>/dev/null | tail -1 || true)"
    if [[ -n "${line}" ]]; then
      # shellcheck disable=SC2086
      eval "${line#export }"
      export region 2>/dev/null || true
    fi
  fi
}

claw_region_name() {
  claw_region_load
  local r="${region:-${REGION:-${CLAW_REGION:-}}}"
  printf '%s' "$(printf '%s' "${r}" | tr '[:upper:]' '[:lower:]')"
}

claw_region_is_china() {
  [[ "$(claw_region_name)" == china ]]
}

# True when CN apt/cargo/rust mirrors should be used (CI SG sets GITHUB_ACTIONS). Author: kejiqing
claw_cn_mirror_enabled() {
  [[ "${GITHUB_ACTIONS:-}" == "true" ]] && return 1
  case "${CLAW_USE_CN_APT_MIRROR:-}" in 1) return 0 ;; 0) return 1 ;; esac
  [[ "${CLAW_USE_CN_CRATES_MIRROR:-0}" == "1" ]] && return 0
  [[ "${CLAW_USE_CN_RUST_MIRROR:-0}" == "1" ]] && return 0
  claw_region_is_china
}

# Apply mirror/registry defaults from region (explicit .env values win). Author: kejiqing
claw_apply_region_defaults() {
  claw_region_load
  if claw_region_is_china; then
    export CLAW_USE_CN_CRATES_MIRROR="${CLAW_USE_CN_CRATES_MIRROR:-1}"
    export CLAW_USE_CN_RUST_MIRROR="${CLAW_USE_CN_RUST_MIRROR:-1}"
    export CLAW_USE_CN_APT_MIRROR="${CLAW_USE_CN_APT_MIRROR:-1}"
    if [[ "${CLAW_USE_DOCKER_IO:-}" != "1" && "${GITHUB_ACTIONS:-}" != "true" ]]; then
      export CONTAINER_BASE_REGISTRY="${CONTAINER_BASE_REGISTRY:-docker.1ms.run}"
    fi
  else
    export CLAW_USE_CN_CRATES_MIRROR="${CLAW_USE_CN_CRATES_MIRROR:-0}"
    export CLAW_USE_CN_RUST_MIRROR="${CLAW_USE_CN_RUST_MIRROR:-0}"
    export CLAW_USE_CN_APT_MIRROR="${CLAW_USE_CN_APT_MIRROR:-0}"
  fi
}

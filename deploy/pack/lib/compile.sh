#!/usr/bin/env bash
# ONE path. Author: kejiqing. Do not invent forks.
# Thin wrapper: optional bin filter via CLAW_PACK_BINS.
# shellcheck shell=bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
# shellcheck source=/dev/null
source "$ROOT/deploy/stack/lib/compose-include.sh"
# shellcheck source=/dev/null
source "$ROOT/deploy/stack/lib/claw-region.sh"
# shellcheck source=/dev/null
source "$ROOT/deploy/stack/lib/rust-compile-image.sh"
# shellcheck source=/dev/null
source "$ROOT/deploy/stack/lib/linux-compile.sh"
# shellcheck source=/dev/null
source "$ROOT/deploy/stack/rust-version.env"

claw_pack_compile() {
  local bins="${CLAW_PACK_BINS:-claw http-gateway-rs neuro-opencode neuro-appserver}"
  export CLAW_LINUX_RELEASE_BINS="$bins"
  export CLAW_LINUX_COMPILE_PLATFORM="${CLAW_LINUX_COMPILE_PLATFORM:-linux/amd64}"
  case "${CLAW_LINUX_COMPILE_PLATFORM}" in
    *arm64*|*aarch64*) export TARGETARCH="${TARGETARCH:-arm64}" ;;
    *) export TARGETARCH="${TARGETARCH:-amd64}" ;;
  esac

  local cli reg compile_image cn_flag=0
  cli="$(claw_container_runtime_cli)" || return 1
  claw_region_load
  if claw_region_is_china; then
    export CLAW_USE_CN_APT_MIRROR="${CLAW_USE_CN_APT_MIRROR:-1}"
    export CLAW_USE_CN_CRATES_MIRROR="${CLAW_USE_CN_CRATES_MIRROR:-1}"
    export CLAW_USE_CN_RUST_MIRROR="${CLAW_USE_CN_RUST_MIRROR:-1}"
    reg="${CONTAINER_BASE_REGISTRY:-docker.m.daocloud.io}"
  else
    export CLAW_USE_CN_APT_MIRROR="${CLAW_USE_CN_APT_MIRROR:-0}"
    export CLAW_USE_CN_CRATES_MIRROR="${CLAW_USE_CN_CRATES_MIRROR:-0}"
    export CLAW_USE_CN_RUST_MIRROR="${CLAW_USE_CN_RUST_MIRROR:-0}"
    reg="${CONTAINER_BASE_REGISTRY:-docker.io}"
  fi
  export CONTAINER_BASE_REGISTRY="$reg"
  compile_image="$(claw_ensure_rust_compile_image "$ROOT" "$cli" "$reg")"
  export CLAW_CONTAINER_RUNTIME=docker
  export CLAW_LINUX_COMPILE_CI=1
  export CLAW_RUST_COMPILE_IMAGE="$compile_image"
  claw_cn_mirror_enabled && cn_flag=1
  claw_linux_compile_release "$ROOT" "$cli" "$compile_image" "$cn_flag"
}

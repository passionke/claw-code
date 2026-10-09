#!/usr/bin/env bash
# ONE path. Author: kejiqing. Do not invent forks.
# Build+push http-gateway-rs + http-gateway-playground.
# shellcheck shell=bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
# shellcheck source=/dev/null
source "$ROOT/deploy/pack/lib/prefix.sh"
# shellcheck source=/dev/null
source "$ROOT/deploy/stack/lib/claw-region.sh"
# shellcheck source=/dev/null
source "$ROOT/deploy/stack/lib/compose-include.sh"

claw_pack_image_gateway() {
  local only="${1:-all}" # all|gateway|playground
  local tag="${RELEASE_TAG:?RELEASE_TAG required}"
  local sha12 prefix platform reg debian_image node_image apt_cn
  sha12="$(git -C "$ROOT" rev-parse --short=12 HEAD)"
  prefix="$(claw_pack_prefix)"
  platform="${CLAW_LINUX_COMPILE_PLATFORM:-linux/amd64}"
  claw_region_load
  if claw_region_is_china; then
    reg="${CONTAINER_BASE_REGISTRY:-docker.m.daocloud.io}"
    apt_cn=1
  else
    reg="${CONTAINER_BASE_REGISTRY:-docker.io}"
    apt_cn=0
  fi
  debian_image="${reg%/}/library/debian:bookworm-slim"
  node_image="${reg%/}/library/node:20-alpine"

  claw_pack_registry_login

  if [[ "$only" == "all" || "$only" == "gateway" ]]; then
    if [[ ! -x "$ROOT/deploy/stack/.linux-artifacts/release/http-gateway-rs" ]]; then
      echo "error: missing http-gateway-rs binary (run compile with CLAW_PACK_BINS including http-gateway-rs)" >&2
      return 1
    fi
    local image="${prefix}/http-gateway-rs"
    "$ROOT/deploy/stack/lib/container-build.sh" docker \
      "$ROOT/deploy/stack/Containerfile.gateway-rs.prebuilt" \
      --platform "$platform" \
      --build-arg "DEBIAN_BASE_IMAGE=${debian_image}" \
      --build-arg "CLAW_USE_CN_APT_MIRROR=${apt_cn}" \
      -t "${image}:${tag}" \
      -t "${image}:sha-${sha12}" \
      -t "${image}:latest"
    claw_pack_skopeo_push "${image}:${tag}" "${image}:${tag}" "${image}:sha-${sha12}" "${image}:latest"
    echo "published ${image}:${tag}"
  fi

  if [[ "$only" == "all" || "$only" == "playground" ]]; then
    local pimg="${prefix}/http-gateway-playground"
    "$ROOT/deploy/stack/lib/container-build.sh" docker \
      "$ROOT/deploy/stack/Containerfile.gateway-playground" \
      --platform "$platform" \
      --build-arg "DEBIAN_BASE_IMAGE=${debian_image}" \
      --build-arg "NODE_BASE_IMAGE=${node_image}" \
      --build-arg "CLAW_USE_CN_APT_MIRROR=${apt_cn}" \
      -t "${pimg}:${tag}" \
      -t "${pimg}:sha-${sha12}" \
      -t "${pimg}:latest"
    claw_pack_skopeo_push "${pimg}:${tag}" "${pimg}:${tag}" "${pimg}:sha-${sha12}" "${pimg}:latest"
    docker run --rm "${pimg}:${tag}" \
      sh -c 'test -f /app/admin-dist/index.html && test -n "$(ls /app/admin-dist/assets/*.js 2>/dev/null)"'
    echo "published ${pimg}:${tag}"
  fi
}

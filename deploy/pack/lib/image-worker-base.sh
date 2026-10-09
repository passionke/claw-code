#!/usr/bin/env bash
# ONE path. Author: kejiqing. Do not invent forks.
# Build+push claw-worker-base (+ relaxed). No CLI binaries inside.
# shellcheck shell=bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
# shellcheck source=/dev/null
source "$ROOT/deploy/pack/lib/prefix.sh"
# shellcheck source=/dev/null
source "$ROOT/deploy/stack/lib/claw-region.sh"

claw_pack_image_worker_base() {
  local tag="${RELEASE_TAG:?RELEASE_TAG required}"
  local sha12 prefix platform reg debian_image apt_cn
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

  claw_pack_registry_login

  local base="${prefix}/claw-worker-base"
  "$ROOT/deploy/stack/lib/container-build.sh" docker \
    "$ROOT/deploy/stack/Containerfile.worker-base" \
    --platform "$platform" \
    --build-arg "DEBIAN_BASE_IMAGE=${debian_image}" \
    --build-arg "CLAW_USE_CN_APT_MIRROR=${apt_cn}" \
    -t "${base}:${tag}" \
    -t "${base}:sha-${sha12}" \
    -t "${base}:latest"
  claw_pack_skopeo_push "${base}:${tag}" "${base}:${tag}" "${base}:sha-${sha12}" "${base}:latest"

  # S3 gate: no claw in image
  if docker run --rm --entrypoint sh "${base}:${tag}" -c 'command -v claw'; then
    echo "error: claw-worker-base must not contain claw" >&2
    return 1
  fi

  local relaxed="${prefix}/claw-worker-base-relaxed"
  "$ROOT/deploy/stack/lib/container-build.sh" docker \
    "$ROOT/deploy/stack/Containerfile.worker-base-relaxed" \
    --platform "$platform" \
    --build-arg "WORKER_BASE_IMAGE=${base}:${tag}" \
    --build-arg "CLAW_USE_CN_APT_MIRROR=${apt_cn}" \
    -t "${relaxed}:${tag}" \
    -t "${relaxed}:sha-${sha12}" \
    -t "${relaxed}:latest"
  claw_pack_skopeo_push "${relaxed}:${tag}" "${relaxed}:${tag}" "${relaxed}:sha-${sha12}" "${relaxed}:latest"
  docker run --rm --entrypoint sh "${relaxed}:${tag}" \
    -c 'command -v curl && command -v git && command -v python3'
  if docker run --rm --entrypoint sh "${relaxed}:${tag}" -c 'command -v claw'; then
    echo "error: claw-worker-base-relaxed must not contain claw" >&2
    return 1
  fi
  echo "published ${base}:${tag} and ${relaxed}:${tag}"
}

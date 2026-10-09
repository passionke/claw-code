#!/usr/bin/env bash
# ONE path. Author: kejiqing. Do not invent forks.
# Push claw / neuro / ACP engine packages as OCI images under claw-cli/*.
# shellcheck shell=bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
# shellcheck source=/dev/null
source "$ROOT/deploy/pack/lib/prefix.sh"
# shellcheck source=/dev/null
source "$ROOT/deploy/stack/lib/claw-region.sh"

_claw_pack_push_staged() {
  local name="$1" # e.g. claw-cli/claw
  local staging="$2"
  local tag="${RELEASE_TAG:?RELEASE_TAG required}"
  local sha12 prefix platform reg debian_image
  sha12="$(git -C "$ROOT" rev-parse --short=12 HEAD)"
  prefix="$(claw_pack_prefix)"
  platform="${CLAW_LINUX_COMPILE_PLATFORM:-linux/amd64}"
  claw_region_load
  if claw_region_is_china; then
    reg="${CONTAINER_BASE_REGISTRY:-docker.m.daocloud.io}"
  else
    reg="${CONTAINER_BASE_REGISTRY:-docker.io}"
  fi
  debian_image="${reg%/}/library/debian:bookworm-slim"
  local image="${prefix}/${name}"
  # Build from staging as context (contains root/ + Containerfile copy)
  cp "$ROOT/deploy/stack/Containerfile.cli-artifact" "${staging}/Containerfile"
  docker build --platform "$platform" \
    --build-arg "DEBIAN_BASE_IMAGE=${debian_image}" \
    -f "${staging}/Containerfile" \
    -t "${image}:${tag}" \
    -t "${image}:sha-${sha12}" \
    -t "${image}:latest" \
    "${staging}"
  claw_pack_skopeo_push "${image}:${tag}" "${image}:${tag}" "${image}:sha-${sha12}" "${image}:latest"
  local digest
  digest="$(skopeo inspect --format '{{.Digest}}' "docker://${image}:${tag}" 2>/dev/null || true)"
  echo "published ${image}:${tag} digest=${digest:-unknown}"
  printf '%s\n' "${digest:-}"
}

claw_pack_artifact_cli() {
  local which="${1:-all}" # all|claw|neuro|acp
  local tag="${RELEASE_TAG:?RELEASE_TAG required}"
  local art="$ROOT/deploy/stack/.linux-artifacts/release"
  claw_pack_registry_login

  if [[ "$which" == "all" || "$which" == "claw" ]]; then
    [[ -x "${art}/claw" ]] || { echo "error: missing ${art}/claw" >&2; return 1; }
    local st
    st="$(mktemp -d)"
    mkdir -p "${st}/root/usr/local/bin"
    cp "${art}/claw" "${st}/root/usr/local/bin/claw"
    chmod 0755 "${st}/root/usr/local/bin/claw"
    _claw_pack_push_staged "claw-cli/claw" "$st"
    rm -rf "$st"
  fi

  if [[ "$which" == "all" || "$which" == "neuro" ]]; then
    for eng in opencode appserver; do
      [[ -x "${art}/neuro-${eng}" ]] || { echo "error: missing ${art}/neuro-${eng}" >&2; return 1; }
      local st
      st="$(mktemp -d)"
      mkdir -p "${st}/root/usr/local/bin"
      cp "${art}/neuro-${eng}" "${st}/root/usr/local/bin/neuro-${eng}"
      chmod 0755 "${st}/root/usr/local/bin/neuro-${eng}"
      _claw_pack_push_staged "claw-cli/neuro-${eng}" "$st"
      rm -rf "$st"
    done
  fi

  if [[ "$which" == "all" || "$which" == "acp" ]]; then
    # ONE path: reuse worker Containerfile fetch stages (pins live there only). Author: kejiqing
    claw_apply_region_defaults
    local npm_reg platform targetarch node_image reg fetch_tag cid st
    npm_reg="$(claw_npm_registry)"
    platform="${CLAW_LINUX_COMPILE_PLATFORM:-linux/amd64}"
    case "$platform" in
      *arm64*|*aarch64*) targetarch=arm64 ;;
      *) targetarch=amd64 ;;
    esac
    if claw_region_is_china; then
      reg="${CONTAINER_BASE_REGISTRY:-docker.1ms.run}"
    else
      reg="${CONTAINER_BASE_REGISTRY:-docker.io}"
    fi
    node_image="${reg%/}/library/node:22-bookworm-slim"

    st="$(mktemp -d)"
    mkdir -p "${st}/root/usr/local/lib/neuro-engines/opencode/bin"
    fetch_tag="claw-pack-acp-opencode-fetch:$$"
    docker build --platform "$platform" \
      --target opencode-fetch \
      -f "$ROOT/deploy/stack/Containerfile.gateway-worker-opencode" \
      --build-arg "NODE_BASE_IMAGE=${node_image}" \
      --build-arg "NPM_REGISTRY=${npm_reg}" \
      --build-arg "TARGETARCH=${targetarch}" \
      --build-arg "WORKER_BASE_IMAGE=${reg%/}/library/debian:bookworm-slim" \
      -t "$fetch_tag" \
      "$ROOT"
    cid="$(docker create "$fetch_tag")"
    docker cp "${cid}:/fetch/package/bin/opencode" \
      "${st}/root/usr/local/lib/neuro-engines/opencode/bin/opencode"
    docker rm -f "$cid" >/dev/null
    docker rmi "$fetch_tag" >/dev/null 2>&1 || true
    chmod 0755 "${st}/root/usr/local/lib/neuro-engines/opencode/bin/opencode"
    _claw_pack_push_staged "claw-cli/acp-opencode" "$st"
    rm -rf "$st"

    st="$(mktemp -d)"
    mkdir -p "${st}/root/usr/local/lib/neuro-engines/codex-acp"
    fetch_tag="claw-pack-acp-appserver-fetch:$$"
    docker build --platform "$platform" \
      --target codex-install \
      -f "$ROOT/deploy/stack/Containerfile.gateway-worker-appserver" \
      --build-arg "NODE_BASE_IMAGE=${node_image}" \
      --build-arg "NPM_REGISTRY=${npm_reg}" \
      --build-arg "WORKER_BASE_IMAGE=${reg%/}/library/debian:bookworm-slim" \
      -t "$fetch_tag" \
      "$ROOT"
    cid="$(docker create "$fetch_tag")"
    docker cp "${cid}:/usr/local/lib/neuro-engines/codex-acp/." \
      "${st}/root/usr/local/lib/neuro-engines/codex-acp/"
    docker rm -f "$cid" >/dev/null
    docker rmi "$fetch_tag" >/dev/null 2>&1 || true
    test -x "${st}/root/usr/local/lib/neuro-engines/codex-acp/node_modules/.bin/codex-acp"
    _claw_pack_push_staged "claw-cli/acp-appserver" "$st"
    rm -rf "$st"
  fi
}

# Pull artifact image layers to a host directory (for inject / inspect). Author: kejiqing
claw_pack_artifact_pull() {
  local image_ref="$1"
  local dest="$2"
  mkdir -p "$dest"
  python3 "$ROOT/deploy/e2b/registry_extract.py" --tree "$image_ref" /usr/local "$dest"
}

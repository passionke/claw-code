#!/usr/bin/env bash
# ONE path. Author: kejiqing. Do not invent forks.
# Register thin worker-base images with e2b via from_image only (no COPY claw).
# shellcheck shell=bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
# shellcheck source=/dev/null
source "$ROOT/deploy/pack/lib/prefix.sh"
# shellcheck source=/dev/null
source "$ROOT/deploy/stack/lib/claw-region.sh"

claw_pack_e2b_register_base() {
  local tag="${1:-${RELEASE_TAG:-}}"
  if [[ -z "$tag" ]]; then
    echo "usage: e2b-register-base <tag>" >&2
    return 2
  fi
  local prefix
  prefix="$(claw_pack_prefix)"
  export CLAW_IMAGE_RELEASE_TAG="$tag"
  export CLAW_E2B_TEMPLATE_BUILD_STRATEGY=from_image
  export CLAW_E2B_WORKER_SKIP_LOCAL_BUILD=0
  # Point at thin base — not claw-gateway-worker.
  export CLAW_E2B_WORKER_IMAGE="${CLAW_E2B_WORKER_IMAGE:-${prefix}/claw-worker-base:${tag}}"
  export CLAW_E2B_TEMPLATE_FROM_IMAGE="${CLAW_E2B_WORKER_IMAGE}"
  export CLAW_E2B_WORKER_RELAXED_IMAGE="${CLAW_E2B_WORKER_RELAXED_IMAGE:-${prefix}/claw-worker-base-relaxed:${tag}}"
  export CLAW_E2B_WORKER_RELAXED_FROM_IMAGE=1
  export CLAW_E2B_TEMPLATE_SKIP_VERIFY="${CLAW_E2B_TEMPLATE_SKIP_VERIFY:-1}"
  # Do NOT build engine templates.
  export CLAW_E2B_SKIP_ENGINE_TEMPLATES=1
  # home-ubt / Jenkins agents use docker, not podman. Author: kejiqing
  if [[ -z "${CLAW_CONTAINER_RUNTIME:-}" ]]; then
    if command -v docker >/dev/null 2>&1; then
      export CLAW_CONTAINER_RUNTIME=docker
    elif command -v podman >/dev/null 2>&1; then
      export CLAW_CONTAINER_RUNTIME=podman
    fi
  fi

  local py="${CLAW_E2B_VENV:-}/bin/python3"
  if [[ ! -x "$py" ]]; then
    py="$(command -v python3)"
  fi
  if ! "$py" -c 'import e2b' 2>/dev/null; then
    echo "error: python missing e2b SDK (set CLAW_E2B_VENV; see deploy/e2b/requirements-e2b-sdk.txt)" >&2
    return 1
  fi
  echo "==> e2b from_image register base=${CLAW_E2B_WORKER_IMAGE} runtime=${CLAW_CONTAINER_RUNTIME:-?}" >&2
  "$py" "$ROOT/deploy/e2b/build-claw-worker-selfhosted.py"
  "$py" "$ROOT/deploy/e2b/build-claw-worker-relaxed-selfhosted.py"
  # nas-api still needed for cluster; keep that script (no claw COPY of solve CLI).
  "$py" "$ROOT/deploy/e2b/build-claw-nas-api-selfhosted.py"
  echo "OK: e2b shell templates registered for tag=${tag} (no engine templates)" >&2
}

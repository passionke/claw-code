#!/usr/bin/env bash
# ONE path. Author: kejiqing. Do not invent forks.
#
# Unique pack entry for Jenkins (code.passionke.top) and GitHub Actions self-hosted.
# Usage:
#   RELEASE_TAG=v1.2.3 CLAW_IMAGE_PREFIX=host/ns ./deploy/pack/publish.sh <track>
# Tracks:
#   gateway [--only playground|gateway]
#   worker-base
#   cli | cli-claw | cli-neuro | cli-acp
#   e2b-register   (from_image shell only; needs E2B_* env)
#   all
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=/dev/null
source "$ROOT/deploy/pack/lib/prefix.sh"
# shellcheck source=/dev/null
source "$ROOT/deploy/pack/lib/compile.sh"
# shellcheck source=/dev/null
source "$ROOT/deploy/pack/lib/image-gateway.sh"
# shellcheck source=/dev/null
source "$ROOT/deploy/pack/lib/image-worker-base.sh"
# shellcheck source=/dev/null
source "$ROOT/deploy/pack/lib/artifact-cli.sh"
# shellcheck source=/dev/null
source "$ROOT/deploy/pack/lib/e2b-register-base.sh"
# shellcheck source=/dev/null
source "$ROOT/deploy/stack/lib/claw-region.sh"

usage() {
  cat <<'EOF'
usage: deploy/pack/publish.sh <track> [options]

tracks:
  gateway [--only playground|gateway]   http-gateway-rs + http-gateway-playground
  worker-base                           claw-worker-base (+ relaxed)
  cli | cli-claw | cli-neuro | cli-acp  Worker CLI artifacts (same track, clipped)
  e2b-register                          from_image register empty shells to e2b
  all                                   gateway + worker-base + cli

env:
  RELEASE_TAG / GIT_TAG                 required (vX.Y.Z or release-v*)
  CLAW_IMAGE_PREFIX                     registry/namespace (or NEXUS_PUSH_REGISTRY+NEXUS_NS)
  CLAW_REGISTRY_USER/PASSWORD           (or NEXUS_USER/PASSWORD)
  REGION=china|else
EOF
}

TRACK="${1:-}"
if [[ -z "$TRACK" || "$TRACK" == "-h" || "$TRACK" == "--help" ]]; then
  usage
  exit 0
fi
shift || true

RELEASE_TAG="${RELEASE_TAG:-${GIT_TAG:-}}"
if [[ -z "$RELEASE_TAG" ]]; then
  RELEASE_TAG="$(git -C "$ROOT" describe --tags --exact-match HEAD 2>/dev/null || true)"
fi
if [[ ! "$RELEASE_TAG" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ && ! "$RELEASE_TAG" =~ ^release-v[0-9] ]]; then
  echo "RELEASE_TAG must match vX.Y.Z or release-v*, got: ${RELEASE_TAG:-<empty>}" >&2
  exit 1
fi
export RELEASE_TAG
export REGION="${REGION:-china}"
claw_region_load

case "$TRACK" in
  gateway)
    only=all
    while [[ $# -gt 0 ]]; do
      case "$1" in
        --only) only="${2:?}"; shift 2 ;;
        *) echo "unknown option: $1" >&2; exit 2 ;;
      esac
    done
    if [[ "$only" == "all" || "$only" == "gateway" ]]; then
      export CLAW_PACK_BINS="http-gateway-rs claw"
      claw_pack_compile
    fi
    claw_pack_image_gateway "$only"
    ;;
  worker-base)
    claw_pack_image_worker_base
    ;;
  cli)
    export CLAW_PACK_BINS="claw neuro-opencode neuro-appserver"
    claw_pack_compile
    claw_pack_artifact_cli all
    ;;
  cli-claw)
    export CLAW_PACK_BINS="claw"
    claw_pack_compile
    claw_pack_artifact_cli claw
    ;;
  cli-neuro)
    export CLAW_PACK_BINS="neuro-opencode neuro-appserver"
    claw_pack_compile
    claw_pack_artifact_cli neuro
    ;;
  cli-acp)
    claw_pack_artifact_cli acp
    ;;
  e2b-register)
    claw_pack_e2b_register_base "$RELEASE_TAG"
    ;;
  all)
    export CLAW_PACK_BINS="claw http-gateway-rs neuro-opencode neuro-appserver"
    claw_pack_compile
    claw_pack_image_gateway all
    claw_pack_image_worker_base
    claw_pack_artifact_cli all
    ;;
  *)
    echo "unknown track: $TRACK" >&2
    usage
    exit 2
    ;;
esac

echo "OK: publish track=${TRACK} tag=${RELEASE_TAG} prefix=$(claw_pack_prefix)"

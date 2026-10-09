#!/usr/bin/env bash
# Register already-published e2b Worker protocol images. Author: kejiqing
# This script never builds Gateway/Admin or Agent engines.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=/dev/null
source "$ROOT/deploy/stack/lib/release-images.sh"

TAG="${1:-${CLAW_IMAGE_RELEASE_TAG:-}}"
if [[ -z "$TAG" ]]; then
  echo "usage: $0 <e2b-protocol-tag>" >&2
  exit 2
fi
PREFIX="$(claw_image_registry_prefix_from_env)"

export CLAW_E2B_TEMPLATE_BUILD_STRATEGY=from_image
export CLAW_E2B_WORKER_IMAGE="${CLAW_E2B_WORKER_IMAGE:-${PREFIX}/debian-bookworm-claw-worker:${TAG}}"
export CLAW_E2B_TEMPLATE_FROM_IMAGE="$CLAW_E2B_WORKER_IMAGE"
export CLAW_E2B_WORKER_RELAXED_IMAGE="${CLAW_E2B_WORKER_RELAXED_IMAGE:-${PREFIX}/debian-bookworm-claw-worker-relaxed:${TAG}}"

export E2B_API_KEY="${E2B_API_KEY:-${CLAW_E2B_API_KEY:-}}"
export E2B_API_URL="${E2B_API_URL:-${CLAW_E2B_API_URL:-}}"
export E2B_SANDBOX_URL="${E2B_SANDBOX_URL:-${CLAW_E2B_SANDBOX_URL:-}}"
export E2B_DOMAIN="${E2B_DOMAIN:-${CLAW_E2B_DOMAIN:-}}"
: "${E2B_API_KEY:?set CLAW_E2B_API_KEY or E2B_API_KEY}"
: "${E2B_API_URL:?set CLAW_E2B_API_URL or E2B_API_URL}"

PYTHON="${CLAW_E2B_VENV:+${CLAW_E2B_VENV}/bin/python3}"
if [[ -z "$PYTHON" || ! -x "$PYTHON" ]]; then
  PYTHON="$(command -v python3)"
fi
"$PYTHON" -c 'import e2b' >/dev/null
"$PYTHON" "$ROOT/deploy/e2b/build-claw-worker-selfhosted.py"
"$PYTHON" "$ROOT/deploy/e2b/build-claw-worker-relaxed-selfhosted.py"

echo "registered e2b Worker protocol tag=${TAG}"

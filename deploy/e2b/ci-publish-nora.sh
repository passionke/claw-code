#!/usr/bin/env bash
# Private Jenkins entry: publish e2b Worker protocol images to Nora. Author: kejiqing
# Never builds Gateway/Admin or Agent engine artifacts.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
bash "$ROOT/deploy/verify-release-boundaries.sh"

RELEASE_TAG="${RELEASE_TAG:-${GIT_TAG:-}}"
if [[ -z "$RELEASE_TAG" ]]; then
  echo "RELEASE_TAG or GIT_TAG is required" >&2
  exit 2
fi

export RELEASE_TAG
export REGION="${REGION:-china}"
export CLAW_IMAGE_PREFIX="${CLAW_IMAGE_PREFIX:-nora.home.passionke.top/passionke}"
export CLAW_LINUX_COMPILE_PLATFORM="${CLAW_LINUX_COMPILE_PLATFORM:-linux/amd64}"

: "${NEXUS_USER:?NEXUS_USER is required (Jenkins nora-deployer)}"
: "${NEXUS_PASSWORD:?NEXUS_PASSWORD is required (Jenkins nora-deployer)}"
export NEXUS_USER NEXUS_PASSWORD
export CLAW_REGISTRY_USER="${CLAW_REGISTRY_USER:-$NEXUS_USER}"
export CLAW_REGISTRY_PASSWORD="${CLAW_REGISTRY_PASSWORD:-$NEXUS_PASSWORD}"

exec bash "$ROOT/deploy/e2b/publish-worker-protocol.sh"

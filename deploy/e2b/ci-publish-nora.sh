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

# Pinned e2b SDK for template register (same pins as GitHub e2b-worker-protocol).
VENV="${CLAW_E2B_VENV:-${ROOT}/.e2b-venv}"
if [[ ! -x "${VENV}/bin/python3" ]]; then
  echo "==> prepare pinned e2b SDK venv at ${VENV}"
  python3 -m venv "$VENV"
  if [[ "$REGION" == "china" ]]; then
    "${VENV}/bin/pip" install \
      -i https://mirrors.aliyun.com/pypi/simple \
      --trusted-host mirrors.aliyun.com \
      -r "$ROOT/deploy/e2b/requirements-e2b-sdk.txt"
  else
    "${VENV}/bin/pip" install -r "$ROOT/deploy/e2b/requirements-e2b-sdk.txt"
  fi
fi
export CLAW_E2B_VENV="$VENV"
"${VENV}/bin/python3" -c 'import e2b'

exec bash "$ROOT/deploy/e2b/publish-worker-protocol.sh"

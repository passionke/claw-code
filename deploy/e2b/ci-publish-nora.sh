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

# home29 agents may lack python3-venv; use --target instead of venv. Author: kejiqing
E2B_PY="${CLAW_E2B_VENV:+${CLAW_E2B_VENV}/bin/python3}"
if [[ -z "${E2B_PY:-}" || ! -x "${E2B_PY}" ]]; then
  PKG_DIR="${ROOT}/.e2b-packages"
  if ! PYTHONPATH="${PKG_DIR}${PYTHONPATH:+:${PYTHONPATH}}" python3 -c 'import e2b' >/dev/null 2>&1; then
    echo "==> install pinned e2b SDK into ${PKG_DIR}"
    mkdir -p "$PKG_DIR"
    PIP_ARGS=(install --upgrade --target "$PKG_DIR" -r "$ROOT/deploy/e2b/requirements-e2b-sdk.txt")
    if [[ "$REGION" == "china" ]]; then
      PIP_ARGS+=(-i https://mirrors.aliyun.com/pypi/simple --trusted-host mirrors.aliyun.com)
    fi
    python3 -m pip "${PIP_ARGS[@]}"
  fi
  export PYTHONPATH="${PKG_DIR}${PYTHONPATH:+:${PYTHONPATH}}"
  # publish-worker-protocol.sh prefers CLAW_E2B_VENV/bin/python3; leave unset so it uses PATH python3.
  unset CLAW_E2B_VENV || true
fi
python3 -c 'import e2b'

exec bash "$ROOT/deploy/e2b/publish-worker-protocol.sh"

#!/usr/bin/env bash
# Build + smoke-check claw-gateway-worker-{opencode,appserver}:<tag> FROM a strict worker image.
# Needs deploy/stack/.linux-artifacts/release/neuro-{opencode,appserver} (linux-compile.sh).
# npm registry follows region (claw_npm_registry): china → npmmirror, else npmjs.
# Usage: build-worker-images.sh <strict-worker-image> <tag>
# Author: kejiqing
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
WORKER_BASE_IMAGE="${1:?usage: $0 <strict-worker-image> <tag>}"
TAG="${2:?usage: $0 <strict-worker-image> <tag>}"

# shellcheck source=/dev/null
source "${ROOT_DIR}/deploy/stack/lib/claw-region.sh"
claw_apply_region_defaults
# shellcheck source=/dev/null
source "${ROOT_DIR}/deploy/stack/lib/compose-include.sh"
CLI="$(claw_container_runtime_cli)"
NPM_REGISTRY="$(claw_npm_registry)"
REG="${CONTAINER_BASE_REGISTRY:-docker.io}"
NODE_BASE_IMAGE="${REG%/}/library/node:22-bookworm-slim"

cd "${ROOT_DIR}"
for bin in neuro-opencode neuro-appserver; do
  if [[ ! -x "deploy/stack/.linux-artifacts/release/${bin}" ]]; then
    echo "error: missing deploy/stack/.linux-artifacts/release/${bin} (run linux-compile.sh)" >&2
    exit 1
  fi
done

echo "==> neuro worker images tag=${TAG} base=${WORKER_BASE_IMAGE}"
echo "    region=$(claw_region_name) npm=${NPM_REGISTRY} node=${NODE_BASE_IMAGE} cli=${CLI}"

engine_smoke() {
  case "$1" in
    opencode) printf '%s' '/usr/local/lib/neuro-engines/opencode/bin/opencode --version' ;;
    appserver)
      printf '%s' 'node --version && test -x /usr/local/lib/neuro-engines/codex-acp/node_modules/.bin/codex-acp && node /usr/local/lib/neuro-engines/codex-acp/node_modules/@openai/codex/bin/codex.js --version'
      ;;
  esac
}

for engine in opencode appserver; do
  image="claw-gateway-worker-${engine}:${TAG}"
  deploy/stack/lib/container-build.sh "${CLI}" "deploy/stack/Containerfile.gateway-worker-${engine}" \
    --build-arg "WORKER_BASE_IMAGE=${WORKER_BASE_IMAGE}" \
    --build-arg "NODE_BASE_IMAGE=${NODE_BASE_IMAGE}" \
    --build-arg "NPM_REGISTRY=${NPM_REGISTRY}" \
    -t "${image}"
  # neuro-* without args prints usage and exits 2: proves the binary loads in this image.
  "${CLI}" run --rm --entrypoint sh "${image}" -c "
    set -eu
    command -v claw
    rc=0; neuro-${engine} >/dev/null 2>&1 || rc=\$?
    [ \"\$rc\" = 2 ] || { echo \"neuro-${engine} exit \$rc (want 2)\" >&2; exit 1; }
    $(engine_smoke "${engine}")
  "
  echo "OK: ${image}"
done

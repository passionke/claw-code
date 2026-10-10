#!/usr/bin/env bash
# Build the codex-acp Agent engine raw artifact. Author: kejiqing
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
VERSION="${CODEX_ACP_VERSION:-2.1.1}"
ARCH="${TARGETARCH:-amd64}"
OUTPUT="${1:-${ROOT}/deploy/agent-engines/dist/codex-acp-${VERSION}-${ARCH}.tar.gz}"
IMAGE="claw-agent-codex-acp-build:${VERSION}-${ARCH}-$$"
STAGE="$(mktemp -d)"
CID=""
cleanup() {
  if [[ -n "$CID" ]]; then
    docker rm -f "$CID" >/dev/null 2>&1 || true
  fi
  docker rmi "$IMAGE" >/dev/null 2>&1 || true
  rm -rf "$STAGE"
}
trap cleanup EXIT

# Keep stdout as a single artifact path for CI capture. Author: kejiqing
docker build \
  --platform "linux/${ARCH}" \
  --build-arg "NPM_REGISTRY=${NPM_REGISTRY:-https://registry.npmjs.org}" \
  -t "$IMAGE" \
  -f "$ROOT/deploy/agent-engines/codex-acp/Containerfile" \
  "$ROOT" >&2
CID="$(docker create "$IMAGE")"
mkdir -p "$STAGE/usr/local/bin" "$STAGE/usr/local/lib/neuro-engines/codex-acp"
docker cp "${CID}:/usr/local/bin/node" "$STAGE/usr/local/bin/node"
docker cp "${CID}:/engine/." "$STAGE/usr/local/lib/neuro-engines/codex-acp/"
chmod 0755 "$STAGE/usr/local/bin/node"
test -x "$STAGE/usr/local/lib/neuro-engines/codex-acp/node_modules/.bin/codex-acp"
mkdir -p "$(dirname "$OUTPUT")"
tar -C "$STAGE" -czf "$OUTPUT" usr
printf '%s\n' "$OUTPUT"

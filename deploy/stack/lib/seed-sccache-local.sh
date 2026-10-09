#!/usr/bin/env bash
# Build local OCI tag sccache:${SCCACHE_VERSION} (static binary). Author: kejiqing
# Used by GHA/ACR (GitHub-reachable download) and as input to seed-sccache-nora.sh.
# Optional: SCCACHE_VERSION REGION=china|else SCCACHE_TGZ_URL
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../../.." && pwd)"
SCCACHE_VERSION="${SCCACHE_VERSION:-v0.10.0}"
REGION="${REGION:-else}"
ARCH_TRIPLE="${SCCACHE_ARCH:-x86_64-unknown-linux-musl}"
LOCAL_TAG="sccache:${SCCACHE_VERSION}"
TGZ_NAME="sccache-${SCCACHE_VERSION}-${ARCH_TRIPLE}.tar.gz"

if docker image inspect "${LOCAL_TAG}" >/dev/null 2>&1; then
  echo "==> reuse local ${LOCAL_TAG}"
  exit 0
fi

if [[ -n "${SCCACHE_TGZ_URL:-}" ]]; then
  TGZ_URL="${SCCACHE_TGZ_URL}"
elif [[ "$REGION" == "china" ]]; then
  TGZ_URL="https://ghfast.top/https://github.com/mozilla/sccache/releases/download/${SCCACHE_VERSION}/${TGZ_NAME}"
else
  TGZ_URL="https://github.com/mozilla/sccache/releases/download/${SCCACHE_VERSION}/${TGZ_NAME}"
fi

CTX="${ROOT}/deploy/stack/.build-ctx/sccache-seed"
rm -rf "${CTX}"
mkdir -p "${CTX}"
cp "${ROOT}/deploy/stack/Containerfile.sccache" "${CTX}/Containerfile"

echo "==> fetch ${TGZ_URL}"
curl --http1.1 -fL --connect-timeout 20 --max-time 180 --retry 3 --retry-delay 2 \
  -o "${CTX}/${TGZ_NAME}" "${TGZ_URL}"
tar xz -C "${CTX}" -f "${CTX}/${TGZ_NAME}"
cp "${CTX}/sccache-${SCCACHE_VERSION}-${ARCH_TRIPLE}/sccache" "${CTX}/sccache"
chmod +x "${CTX}/sccache"
"${CTX}/sccache" --version

echo "==> build ${LOCAL_TAG}"
DOCKER_BUILDKIT="${DOCKER_BUILDKIT:-0}" docker build -f "${CTX}/Containerfile" -t "${LOCAL_TAG}" "${CTX}"
echo "seeded ${LOCAL_TAG}"

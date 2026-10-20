#!/usr/bin/env bash
# Build neuro-harness-acp Agent engine (claw CLI) as Nora raw tar. Author: kejiqing
# Places /usr/local/bin/claw for harnessEngine=claw worker.init install.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
# shellcheck source=/dev/null
source "$ROOT/deploy/stack/lib/compose-include.sh"
# shellcheck source=/dev/null
source "$ROOT/deploy/stack/lib/claw-region.sh"
# shellcheck source=/dev/null
source "$ROOT/deploy/stack/lib/rust-compile-image.sh"
# shellcheck source=/dev/null
source "$ROOT/deploy/stack/lib/linux-compile.sh"

VERSION="${NEURO_HARNESS_ACP_VERSION:-${ENGINE_VERSION:-0.1.0}}"
ARCH="${TARGETARCH:-amd64}"
OUTPUT="${1:-${ROOT}/deploy/agent-engines/dist/neuro-harness-acp-${VERSION}-${ARCH}.tar.gz}"
REGION="${REGION:-china}"
claw_region_load

PLATFORM="${CLAW_LINUX_COMPILE_PLATFORM:-linux/${ARCH}}"
export CLAW_LINUX_COMPILE_PLATFORM="$PLATFORM"
export CLAW_LINUX_RELEASE_BINS="claw"
export CLAW_LINUX_COMPILE_CI="${CLAW_LINUX_COMPILE_CI:-1}"
export CLAW_CONTAINER_RUNTIME=docker
export CLAW_DISABLE_SCCACHE="${CLAW_DISABLE_SCCACHE:-1}"

if claw_region_is_china; then
  export CLAW_USE_CN_APT_MIRROR="${CLAW_USE_CN_APT_MIRROR:-1}"
  export CLAW_USE_CN_CRATES_MIRROR="${CLAW_USE_CN_CRATES_MIRROR:-1}"
  export CLAW_USE_CN_RUST_MIRROR="${CLAW_USE_CN_RUST_MIRROR:-1}"
  BASE_REGISTRY="${CONTAINER_BASE_REGISTRY:-docker.m.daocloud.io}"
else
  export CLAW_USE_CN_APT_MIRROR="${CLAW_USE_CN_APT_MIRROR:-0}"
  export CLAW_USE_CN_CRATES_MIRROR="${CLAW_USE_CN_CRATES_MIRROR:-0}"
  export CLAW_USE_CN_RUST_MIRROR="${CLAW_USE_CN_RUST_MIRROR:-0}"
  BASE_REGISTRY="${CONTAINER_BASE_REGISTRY:-docker.io}"
fi
BASE_REGISTRY="${BASE_REGISTRY%/}"
export CONTAINER_BASE_REGISTRY="$BASE_REGISTRY"

CLI="$(claw_container_runtime_cli)"
COMPILE_IMAGE="$(claw_ensure_rust_compile_image "$ROOT" "$CLI" "$BASE_REGISTRY")"
CN_FLAG=0
claw_cn_mirror_enabled && CN_FLAG=1
claw_linux_compile_release "$ROOT" "$CLI" "$COMPILE_IMAGE" "$CN_FLAG" >&2

BIN="$ROOT/deploy/stack/.linux-artifacts/release/claw"
[[ -x "$BIN" ]] || { echo "missing claw binary at ${BIN}" >&2; exit 1; }

STAGE="$(mktemp -d)"
cleanup() { rm -rf "$STAGE"; }
trap cleanup EXIT
mkdir -p "$STAGE/usr/local/bin"
cp -a "$BIN" "$STAGE/usr/local/bin/claw"
chmod 0755 "$STAGE/usr/local/bin/claw"
mkdir -p "$(dirname "$OUTPUT")"
tar -C "$STAGE" -czf "$OUTPUT" usr
printf '%s\n' "$OUTPUT"

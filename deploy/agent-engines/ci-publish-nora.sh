#!/usr/bin/env bash
# Private Jenkins entry: build one Agent engine and upload to Nora raw. Author: kejiqing
# Never builds Gateway/Admin or e2b Worker protocol.
#
# Required:
#   ENGINE_ID          directory name under deploy/agent-engines/
#   ENGINE_VERSION     artifact version label, e.g. 1.18.34 or 1.18.34-amd64
#   RAW_USERNAME/PASSWORD  Nora deployer (Jenkins credential nora-deployer)
# Engine-specific build knobs (OPENCODE_VERSION / CODEX_ACP_VERSION / …) stay on the
# engine's own build.sh; Jenkins sets them when needed.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
bash "$ROOT/deploy/verify-release-boundaries.sh"

ENGINE_ID="${1:-${ENGINE_ID:-}}"
ENGINE_VERSION="${ENGINE_VERSION:-${VERSION:-}}"
if [[ -z "$ENGINE_ID" || -z "$ENGINE_VERSION" ]]; then
  echo "usage: ENGINE_VERSION=<ver> $0 <engine-id>" >&2
  exit 2
fi
[[ "$ENGINE_ID" =~ ^[a-z0-9][a-z0-9._-]*$ ]] || {
  echo "invalid ENGINE_ID: $ENGINE_ID" >&2
  exit 2
}

BUILD_SH="$ROOT/deploy/agent-engines/${ENGINE_ID}/build.sh"
[[ -f "$BUILD_SH" ]] || {
  echo "missing engine build script: $BUILD_SH" >&2
  exit 1
}

ARCH="${TARGETARCH:-amd64}"
RAW_BASE="${RAW_BASE:-https://nora.home.passionke.top/raw/claw-agent-engines}"
RAW_BASE="${RAW_BASE%/}"
VERSION_LABEL="$ENGINE_VERSION"
if [[ "$VERSION_LABEL" != *"-amd64" && "$VERSION_LABEL" != *"-arm64" ]]; then
  VERSION_LABEL="${VERSION_LABEL}-${ARCH}"
fi

: "${RAW_USERNAME:?RAW_USERNAME is required (Jenkins nora-deployer)}"
: "${RAW_PASSWORD:?RAW_PASSWORD is required (Jenkins nora-deployer)}"

echo "==> build Agent engine ${ENGINE_ID} (${VERSION_LABEL})"
export TARGETARCH="$ARCH"
# Engine build.sh prints only the tar path on stdout.
TAR_PATH="$(bash "$BUILD_SH" | tail -n 1)"
TAR_PATH="${TAR_PATH//$'\r'/}"
[[ -f "$TAR_PATH" ]] || { echo "build produced no tar: ${TAR_PATH:-<empty>}" >&2; exit 1; }

echo "==> upload to Nora raw ${RAW_BASE}"
OUT="$(
  RAW_USERNAME="$RAW_USERNAME" RAW_PASSWORD="$RAW_PASSWORD" \
    bash "$ROOT/deploy/agent-engines/upload-raw.sh" \
      "$TAR_PATH" "$ENGINE_ID" "$VERSION_LABEL" "$RAW_BASE"
)"
printf '%s\n' "$OUT"
REF="$(printf '%s\n' "$OUT" | awk -F= '/^ref=/{print $2; exit}')"
DIGEST="$(printf '%s\n' "$OUT" | awk -F= '/^digest=/{print $2; exit}')"
[[ -n "$REF" && -n "$DIGEST" ]] || { echo "upload-raw.sh missing ref/digest" >&2; exit 1; }

echo "==> verify download + digest"
DL="$(mktemp)"
trap 'rm -f "$DL"' EXIT
curl -fsS --retry 3 --retry-connrefused -o "$DL" "$REF"
if command -v sha256sum >/dev/null 2>&1; then
  GOT="sha256:$(sha256sum "$DL" | awk '{print $1}')"
else
  GOT="sha256:$(shasum -a 256 "$DL" | awk '{print $1}')"
fi
[[ "$GOT" == "$DIGEST" ]] || {
  echo "digest mismatch: got ${GOT}, want ${DIGEST}" >&2
  exit 1
}
tar -tzf "$DL" >/dev/null

echo "published agent engine ${ENGINE_ID}: ${REF} (${DIGEST})"

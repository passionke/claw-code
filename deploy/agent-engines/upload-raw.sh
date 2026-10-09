#!/usr/bin/env bash
# Validate and upload one already-built Agent engine tarball. Author: kejiqing
set -euo pipefail

if [[ "$#" -ne 4 ]]; then
  echo "usage: $0 <engine.tar.gz> <artifact-name> <version> <raw-base-url>" >&2
  exit 2
fi

TAR_PATH="$1"
ARTIFACT="$2"
VERSION="$3"
RAW_BASE="${4%/}"
[[ -f "$TAR_PATH" ]] || { echo "missing tar: $TAR_PATH" >&2; exit 1; }
[[ "$ARTIFACT" =~ ^[a-z0-9][a-z0-9._-]*$ ]] || { echo "invalid artifact name: $ARTIFACT" >&2; exit 1; }
[[ "$VERSION" =~ ^[A-Za-z0-9][A-Za-z0-9._-]*$ ]] || { echo "invalid version: $VERSION" >&2; exit 1; }
[[ "$RAW_BASE" == http://* || "$RAW_BASE" == https://* ]] || {
  echo "raw base URL must be http(s)://: $RAW_BASE" >&2
  exit 1
}

LIST="$(mktemp)"
trap 'rm -f "$LIST"' EXIT
tar -tzf "$TAR_PATH" >"$LIST"
[[ -s "$LIST" ]] || { echo "empty tar: $TAR_PATH" >&2; exit 1; }
while IFS= read -r path; do
  clean="${path#./}"
  case "$clean" in
    usr/ | usr/local/ | usr/local/*) ;;
    *) echo "invalid tar path (must be under usr/local/): $path" >&2; exit 1 ;;
  esac
  [[ "$clean" != /* && "$clean" != *"/../"* && "$clean" != "../"* ]] || {
    echo "unsafe tar path: $path" >&2
    exit 1
  }
done <"$LIST"

if command -v sha256sum >/dev/null 2>&1; then
  SHA256="$(sha256sum "$TAR_PATH" | awk '{print $1}')"
else
  SHA256="$(shasum -a 256 "$TAR_PATH" | awk '{print $1}')"
fi
URL="${RAW_BASE}/${ARTIFACT}-${VERSION}.tar.gz"
AUTH=()
if [[ -n "${RAW_USERNAME:-}" || -n "${RAW_PASSWORD:-}" ]]; then
  : "${RAW_USERNAME:?RAW_USERNAME is required when RAW_PASSWORD is set}"
  : "${RAW_PASSWORD:?RAW_PASSWORD is required when RAW_USERNAME is set}"
  AUTH=(-u "${RAW_USERNAME}:${RAW_PASSWORD}")
fi

curl -fsS --retry 3 --retry-connrefused "${AUTH[@]}" -T "$TAR_PATH" "$URL"
printf 'ref=%s\n' "$URL"
printf 'digest=sha256:%s\n' "$SHA256"

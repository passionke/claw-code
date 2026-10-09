#!/usr/bin/env bash
# Guest worker.init: unpack CLI tar.gz (layout: usr/local/...) at /.
#
# IMAGE_REF:
#   http(s)://.../claw-....tar.gz   Nexus raw
#   file:///path/to/claw.tar.gz     local file (operator places it where the guest can read)
# Author: kejiqing
set -euo pipefail
: "${IMAGE_REF:?IMAGE_REF is required}"

TGZ=""
DOWNLOADED=0
cleanup() { [[ "$DOWNLOADED" == 1 && -n "${TGZ:-}" ]] && rm -f "$TGZ" || true; }
trap cleanup EXIT

case "$IMAGE_REF" in
  file://*)
    TGZ="${IMAGE_REF#file://}"
    # file:///abs → /abs
    [[ "$TGZ" == /* ]] || TGZ="/$TGZ"
    test -f "$TGZ" || { echo "file:// not found: $TGZ" >&2; exit 1; }
    ;;
  http://* | https://*)
    AUTH=()
    if [[ -n "${CLAW_REGISTRY_USER:-${ACR_USERNAME:-${ACR_USER:-}}}" ]]; then
      AUTH=(-u "${CLAW_REGISTRY_USER:-${ACR_USERNAME:-${ACR_USER}}}:${CLAW_REGISTRY_PASSWORD:-${ACR_PASSWORD:-${ACR_PASSWORK:-}}}")
    fi
    TGZ=$(mktemp)
    DOWNLOADED=1
    curl -fsSL --retry 3 "${AUTH[@]}" -o "$TGZ" "$IMAGE_REF"
    ;;
  *)
    echo "IMAGE_REF must be http(s):// or file://, got: $IMAGE_REF" >&2
    exit 1
    ;;
esac

tar -tzf "$TGZ" >/dev/null
tar -xzf "$TGZ" -C /

: "${EXPECTED_PATHS:=/usr/local/bin/claw}"
for dst in $EXPECTED_PATHS; do
  if [[ ! -e "$dst" ]]; then
    echo "missing $dst after unpack of $IMAGE_REF" >&2
    exit 1
  fi
  if [[ -f "$dst" ]]; then
    chmod a+x "$dst" || true
  fi
done

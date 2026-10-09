#!/usr/bin/env bash
# Worker init: download and install exactly one selected Agent engine. Author: kejiqing
set -euo pipefail

: "${ENGINE_REF:?ENGINE_REF is required}"
: "${ENGINE_DIGEST:?ENGINE_DIGEST is required}"
case "$ENGINE_REF" in
  http://* | https://*) ;;
  *) echo "ENGINE_REF must be an http(s) raw tar.gz URL" >&2; exit 1 ;;
esac
[[ "$ENGINE_DIGEST" =~ ^sha256:[0-9a-fA-F]{64}$ ]] || {
  echo "ENGINE_DIGEST must be sha256:<64 hex>" >&2
  exit 1
}

TGZ="$(mktemp)"
LIST="$(mktemp)"
cleanup() { rm -f "$TGZ" "$LIST"; }
trap cleanup EXIT

curl -fsSL --retry 3 --retry-connrefused -o "$TGZ" "$ENGINE_REF"
printf '%s  %s\n' "${ENGINE_DIGEST#sha256:}" "$TGZ" | sha256sum -c -
tar -tzf "$TGZ" >"$LIST"
[[ -s "$LIST" ]] || { echo "Agent engine tar is empty" >&2; exit 1; }
while IFS= read -r path; do
  clean="${path#./}"
  case "$clean" in
    usr/ | usr/local/ | usr/local/*) ;;
    *) echo "invalid Agent engine tar path: $path" >&2; exit 1 ;;
  esac
  [[ "$clean" != /* && "$clean" != *"/../"* && "$clean" != "../"* ]] || {
    echo "unsafe Agent engine tar path: $path" >&2
    exit 1
  }
done <"$LIST"
tar -xzf "$TGZ" -C /

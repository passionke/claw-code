#!/usr/bin/env bash
# Upload one already-built Agent engine tar to the public GitHub Release. Author: kejiqing
# Does not compile. The caller passes the tar from the engine build.sh.
set -euo pipefail

if [[ "$#" -ne 2 ]]; then
  echo "usage: $0 <engine.tar.gz> <release-vX.Y.Z>" >&2
  exit 2
fi

TAR_PATH="$1"
TAG="$2"
REPO="${GH_REPO:-passionke/claw-code}"
: "${GITHUB_TOKEN:?GITHUB_TOKEN is required}"
[[ -f "$TAR_PATH" ]] || { echo "missing tar: $TAR_PATH" >&2; exit 1; }
[[ "$TAG" == release-v* ]] || { echo "tag must be release-v*, got: ${TAG}" >&2; exit 2; }
[[ "$REPO" == "passionke/claw-code" ]] || {
  echo "public engine Release repo is passionke/claw-code, got: ${REPO}" >&2
  exit 2
}

ASSET="$(basename "$TAR_PATH")"
[[ "$ASSET" == *.tar.gz ]] || { echo "asset name must end with .tar.gz: ${ASSET}" >&2; exit 2; }

if command -v sha256sum >/dev/null 2>&1; then
  DIGEST="sha256:$(sha256sum "$TAR_PATH" | awk '{print $1}')"
else
  DIGEST="sha256:$(shasum -a 256 "$TAR_PATH" | awk '{print $1}')"
fi

API="https://api.github.com/repos/${REPO}"
AUTH=(-H "Authorization: Bearer ${GITHUB_TOKEN}" -H "Accept: application/vnd.github+json" -H "X-GitHub-Api-Version: 2022-11-28")

BODY="$(mktemp)"
trap 'rm -f "$BODY"' EXIT
CODE="$(curl -sS -o "$BODY" -w '%{http_code}' "${AUTH[@]}" "${API}/releases/tags/${TAG}" || true)"
if [[ "$CODE" == "404" ]]; then
  CODE="$(curl -sS -o "$BODY" -w '%{http_code}' "${AUTH[@]}" -X POST "${API}/releases" \
    -d "{\"tag_name\":\"${TAG}\",\"name\":\"${TAG}\",\"draft\":false,\"prerelease\":false}" || true)"
fi
[[ "$CODE" == "200" || "$CODE" == "201" ]] || {
  echo "github release ${TAG} failed HTTP ${CODE}" >&2
  cat "$BODY" >&2
  exit 1
}

python3 - "$BODY" "$ASSET" <<'PY' >"${BODY}.ids"
import json, sys
rel = json.load(open(sys.argv[1], encoding="utf-8"))
name = sys.argv[2]
print(rel["id"])
for asset in rel.get("assets") or []:
    if asset.get("name") == name:
        print(asset["id"])
PY
RELEASE_ID="$(head -n 1 "${BODY}.ids")"
ASSET_ID="$(sed -n '2p' "${BODY}.ids" || true)"
rm -f "${BODY}.ids"
[[ -n "$RELEASE_ID" ]] || { echo "github release id missing for ${TAG}" >&2; exit 1; }
if [[ -n "$ASSET_ID" ]]; then
  curl -fsS "${AUTH[@]}" -X DELETE "${API}/releases/assets/${ASSET_ID}" >/dev/null
fi

UPLOAD_CODE="$(curl -sS -o "$BODY" -w '%{http_code}' \
  -H "Authorization: Bearer ${GITHUB_TOKEN}" \
  -H "Accept: application/vnd.github+json" \
  -H "Content-Type: application/gzip" \
  --data-binary @"${TAR_PATH}" \
  "https://uploads.github.com/repos/${REPO}/releases/${RELEASE_ID}/assets?name=${ASSET}" || true)"
[[ "$UPLOAD_CODE" == "201" ]] || {
  echo "upload ${ASSET} failed HTTP ${UPLOAD_CODE}" >&2
  cat "$BODY" >&2
  exit 1
}

echo "ref=https://github.com/${REPO}/releases/download/${TAG}/${ASSET}"
echo "digest=${DIGEST}"

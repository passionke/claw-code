#!/usr/bin/env bash
# Push a local docker-daemon image to ACR as Docker v2s2 (Aliyun personal ACR rejects OCI empty layers).
# Author: kejiqing
#
# Usage:
#   ci-push-acr-skopeo.sh <local_ref> <acr_ref> [more_acr_refs...]
# Example:
#   ci-push-acr-skopeo.sh claw-code:release-v1.2.3 \
#     crpi-xxx.cn-hangzhou.personal.cr.aliyuncs.com/passionke/claw-code:release-v1.2.3 \
#     crpi-xxx.cn-hangzhou.personal.cr.aliyuncs.com/passionke/claw-code:latest
#
# Requires: skopeo, docker login to ACR (writes ~/.docker/config.json).
set -euo pipefail

if [[ $# -lt 2 ]]; then
  echo "usage: $0 <local_ref> <acr_ref> [more_acr_refs...]" >&2
  exit 2
fi

LOCAL_REF="$1"
shift

AUTHFILE="${AUTHFILE:-${RUNNER_TEMP:-/tmp}/docker-auth.json}"
if [[ ! -f "${AUTHFILE}" ]]; then
  if [[ -f "${HOME}/.docker/config.json" ]]; then
    cp "${HOME}/.docker/config.json" "${AUTHFILE}"
  else
    echo "ci-push-acr-skopeo: missing docker auth (${HOME}/.docker/config.json)" >&2
    exit 1
  fi
fi

if ! command -v skopeo >/dev/null 2>&1; then
  echo "ci-push-acr-skopeo: skopeo not found" >&2
  exit 1
fi

SRC="docker-daemon:${LOCAL_REF}"
FIRST=1
PRIMARY=""
for dst in "$@"; do
  DST="docker://${dst}"
  if [[ "${FIRST}" -eq 1 ]]; then
    echo "==> skopeo copy --format v2s2 ${SRC} → ${DST}"
    skopeo copy --authfile "${AUTHFILE}" --format v2s2 "${SRC}" "${DST}"
    PRIMARY="${DST}"
    FIRST=0
  else
    echo "==> skopeo copy --format v2s2 ${PRIMARY} → ${DST}"
    skopeo copy --authfile "${AUTHFILE}" --format v2s2 "${PRIMARY}" "${DST}"
  fi
done

echo "ci-push-acr-skopeo: ok ${LOCAL_REF} → $*"

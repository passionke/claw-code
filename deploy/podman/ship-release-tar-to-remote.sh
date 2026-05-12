#!/usr/bin/env bash
# Pull gateway + worker images for one release tag, save to a single tar, scp to a remote host.
# Remote then: podman load -i ~/claw-release-<tag>.tar   (or docker load -i …)
# Use when GHCR is flaky from the server but you can pull once from a better network. kejiqing
#
# Usage:
#   ./deploy/podman/ship-release-tar-to-remote.sh release-v1.0.25
#   ./deploy/podman/ship-release-tar-to-remote.sh release-v1.0.25 admin@192.168.9.252
#
# Env:
#   CLAW_SHIP_REGISTRY_PREFIX   Image prefix (no tag). Default: ghcr.io/passionke
#                                 Set to your ACR prefix if GHCR pull fails locally, e.g.
#                                 crpi-xxxx.cn-hangzhou.personal.cr.aliyuncs.com/passionke
#   CLAW_CONTAINER_RUNTIME       podman | docker | auto (same as rest of deploy scripts)
#   CLAW_SHIP_SKIP_PULL=1        Skip pull (images must already exist locally)
#   CLAW_SHIP_SKIP_SAVE=1       Only scp an existing ~/… tar (set CLAW_SHIP_TAR path)
#   CLAW_SHIP_TAR                Path to existing tar when using SKIP_SAVE flow
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/../.." && pwd)"

TAG="${1:?usage: $0 <release-tag> [user@host]}"
REMOTE="${2:-admin@192.168.9.252}"

if [[ -f "${ROOT_DIR}/.env" ]]; then
  set -a
  # shellcheck source=/dev/null
  source "${ROOT_DIR}/.env"
  set +a
fi
# shellcheck source=/dev/null
source "${ROOT_DIR}/deploy/podman/compose-include.sh"

CLI="$(claw_container_runtime_cli)" || exit 1
PREFIX="${CLAW_SHIP_REGISTRY_PREFIX:-ghcr.io/passionke}"
PREFIX="${PREFIX%/}"
GATEWAY_IMG="${PREFIX}/claw-code:${TAG}"
WORKER_IMG="${PREFIX}/claw-gateway-worker:${TAG}"
SAFE_TAG="${TAG//\//-}"
TAR_NAME="claw-release-${SAFE_TAG}.tar"
LOCAL_TAR="${CLAW_SHIP_TAR:-${ROOT_DIR}/deploy/podman/${TAR_NAME}}"

if [[ "${CLAW_SHIP_SKIP_SAVE:-0}" != "1" ]]; then
  if [[ "${CLAW_SHIP_SKIP_PULL:-0}" != "1" ]]; then
    echo "pull ${GATEWAY_IMG} …"
    "${CLI}" pull "${GATEWAY_IMG}"
    echo "pull ${WORKER_IMG} …"
    "${CLI}" pull "${WORKER_IMG}"
  else
    echo "CLAW_SHIP_SKIP_PULL=1: assuming images already present locally"
  fi
  echo "save -> ${LOCAL_TAR}"
  "${CLI}" save -o "${LOCAL_TAR}" "${GATEWAY_IMG}" "${WORKER_IMG}"
else
  if [[ ! -f "${LOCAL_TAR}" ]]; then
    echo "error: CLAW_SHIP_SKIP_SAVE=1 but missing tar: ${LOCAL_TAR}" >&2
    exit 1
  fi
fi

REMOTE_PATH="${REMOTE_DIR}/${TAR_NAME}"
# Expand ~ for scp destination (scp wants user@host:path)
REMOTE_TILDE="${REMOTE}:~/$(basename "${TAR_NAME}")"
echo "scp ${LOCAL_TAR} -> ${REMOTE_TILDE}"
scp "${LOCAL_TAR}" "${REMOTE_TILDE}"

echo ""
echo "On ${REMOTE}, load then deploy:"
echo "  ${CLI} load -i ~/${TAR_NAME}    # use docker instead of ${CLI} if that host uses docker"
echo "  cd ~/claw-code && CLAW_IMAGE_PREFIX=${PREFIX} ./deploy/podman/up.sh --release ${TAG}"
echo ""
echo "Local tar kept at: ${LOCAL_TAR}  (rm when done to save disk)"

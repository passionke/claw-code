#!/usr/bin/env bash
# Install host `claw-pool-daemon` binary from the same gateway image as production (GHCR). No Rust on server. Author: kejiqing
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=/dev/null
source "${ROOT}/deploy/podman/compose-include.sh"

CLAW_IMAGE_RELEASE_TAG=""
OUT="/usr/local/bin/claw-pool-daemon"
while [[ $# -gt 0 ]]; do
  case "$1" in
    --release=*)
      CLAW_IMAGE_RELEASE_TAG="${1#*=}"
      shift
      ;;
    --release)
      if [[ $# -lt 2 ]]; then
        echo "error: --release requires a value" >&2
        exit 1
      fi
      CLAW_IMAGE_RELEASE_TAG="$2"
      shift 2
      ;;
    release-v*)
      CLAW_IMAGE_RELEASE_TAG="$1"
      shift
      ;;
    -h | --help)
      echo "usage: $0 [--release <tag>|release-v*] [install_path]" >&2
      echo "  default install_path: /usr/local/bin/claw-pool-daemon" >&2
      exit 0
      ;;
    *)
      OUT="$1"
      shift
      if [[ $# -gt 0 ]]; then
        echo "error: unexpected extra arguments: $*" >&2
        exit 1
      fi
      break
      ;;
  esac
done

if [[ -f "${ROOT}/.env" ]]; then
  set -a
  # shellcheck source=/dev/null
  source "${ROOT}/.env"
  set +a
fi

if [[ -n "${CLAW_IMAGE_RELEASE_TAG:-}" ]]; then
  claw_apply_release_image_tag "${CLAW_IMAGE_RELEASE_TAG}"
fi

IMG="${GATEWAY_IMAGE:?set GATEWAY_IMAGE in .env (e.g. ghcr.io/<owner>/claw-code:release-x.y.z)}"
CLI="$(claw_container_runtime_cli)"
echo "pull ${IMG} (if needed) …" >&2
"${CLI}" pull "${IMG}"
TMP="$(mktemp)"
trap 'rm -f "${TMP}"' EXIT
"${CLI}" run --rm --entrypoint cat "${IMG}" /usr/local/bin/claw-pool-daemon >"${TMP}"
install -m 0755 "${TMP}" "${OUT}"
echo "installed ${OUT}" >&2

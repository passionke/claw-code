#!/usr/bin/env bash
# Legacy: install host `claw-pool-daemon` binary (normal deploy uses compose sidecar; see podman-compose.pool-rpc.yml). Author: kejiqing
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
# shellcheck source=/dev/null
source "${ROOT}/deploy/stack/lib/compose-include.sh"

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

# gateway.sh up may already export a release-pinned GATEWAY_IMAGE; do not let .env :local override it.
_pinned_gw=""
if [[ "${GATEWAY_IMAGE:-}" == *claw-code* ]]; then
  _pinned_gw="${GATEWAY_IMAGE}"
fi
if [[ -f "${ROOT}/.env" ]]; then
  set -a
  # shellcheck source=/dev/null
  source "${ROOT}/.env"
  set +a
fi
if [[ -n "${_pinned_gw}" ]]; then
  export GATEWAY_IMAGE="${_pinned_gw}"
elif [[ -n "${CLAW_IMAGE_RELEASE_TAG:-}" ]]; then
  claw_apply_release_image_tag "${CLAW_IMAGE_RELEASE_TAG}"
fi

IMG="${GATEWAY_IMAGE:?set GATEWAY_IMAGE in .env or run gateway.sh up --release <tag>}"
CLI="$(claw_container_runtime_cli)"
if [[ -f "${OUT}" ]] && file "${OUT}" 2>/dev/null | grep -q "Mach-O"; then
  echo "skip pool-daemon install: ${OUT} is a macOS binary (gateway image carries Linux); keep host build" >&2
  exit 0
fi
echo "pull ${IMG} (if needed) …" >&2
if ! "${CLI}" image exists "${IMG}" 2>/dev/null; then
  "${CLI}" pull "${IMG}"
else
  echo "image ${IMG} already present locally, skip pull" >&2
fi
TMP="$(mktemp)"
trap 'rm -f "${TMP}"' EXIT
mkdir -p "$(dirname "${OUT}")"
"${CLI}" run --rm --entrypoint cat "${IMG}" /usr/local/bin/claw-pool-daemon >"${TMP}"
install -m 0755 "${TMP}" "${OUT}"
echo "installed ${OUT}" >&2

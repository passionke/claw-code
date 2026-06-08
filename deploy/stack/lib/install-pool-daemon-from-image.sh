#!/usr/bin/env bash
# Install host `claw-pool-daemon` binary from gateway image (v1: host pool on 9944). Author: kejiqing
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

# shellcheck source=/dev/null
source "${ROOT}/deploy/stack/lib/pool-daemon-binary.sh"

# gateway.sh up may already export a release-pinned GATEWAY_IMAGE; do not let .env :local override it.
_pinned_gw=""
if claw_gateway_image_carries_pool_daemon "${GATEWAY_IMAGE:-}"; then
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

is_local_tag() {
  # pack-deploy local builds claw-gateway-rs:local / claw-gateway-worker:local locally.
  # When local tag image isn't found in the current runtime, pulling from remote may fail
  # (and isn't logically correct for :local). kejiqing
  [[ "${IMG}" == *":local" ]]
}

if [[ -f "${OUT}" ]] && file "${OUT}" 2>/dev/null | grep -q "Mach-O"; then
  echo "skip pool-daemon install: ${OUT} is a macOS binary (gateway image carries Linux); keep host build" >&2
  exit 0
fi

echo "resolve host pool-daemon binary from ${IMG} …" >&2

try_cli() {
  local c="$1"
  if command -v "${c}" >/dev/null 2>&1; then
    if [[ "${c}" == docker ]]; then
      "${c}" image inspect "${IMG}" >/dev/null 2>&1 && return 0
    else
      "${c}" image exists "${IMG}" >/dev/null 2>&1 && return 0
    fi
  fi
  return 1
}

ALT_CLI=""
case "${CLI}" in
  docker) ALT_CLI="podman" ;;
  podman) ALT_CLI="docker" ;;
esac

if try_cli "${CLI}"; then
  :
elif [[ -n "${ALT_CLI}" ]] && try_cli "${ALT_CLI}"; then
  CLI="${ALT_CLI}"
else
  if is_local_tag; then
    echo "error: ${IMG} not found in local ${CLI} (and ${ALT_CLI:-none}) image store; refusing remote pull for :local" >&2
    echo "hint: ensure CLAW_CONTAINER_RUNTIME matches the runtime used during pack-deploy build (docker vs podman)" >&2
    return 1
  fi
  echo "pull ${IMG} (if needed) …" >&2
  "${CLI}" pull "${IMG}" >&2
fi

echo "using ${CLI} to extract ${IMG}" >&2
TMP="$(mktemp)"
trap 'rm -f "${TMP}"' EXIT
mkdir -p "$(dirname "${OUT}")"
"${CLI}" run --rm --entrypoint cat "${IMG}" /usr/local/bin/claw-pool-daemon >"${TMP}"
install -m 0755 "${TMP}" "${OUT}"
echo "installed ${OUT}" >&2

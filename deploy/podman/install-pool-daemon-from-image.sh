#!/usr/bin/env bash
# Install host `claw-pool-daemon` binary from the same gateway image as production (GHCR). No Rust on server. Author: kejiqing
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=/dev/null
source "${ROOT}/deploy/podman/compose-include.sh"
if [[ -f "${ROOT}/.env" ]]; then
  set -a
  # shellcheck source=/dev/null
  source "${ROOT}/.env"
  set +a
fi
IMG="${GATEWAY_IMAGE:?set GATEWAY_IMAGE in .env (e.g. ghcr.io/<owner>/claw-code:release-x.y.z)}"
OUT="${1:-/usr/local/bin/claw-pool-daemon}"
CLI="$(claw_container_runtime_cli)"
if [[ "${CLAW_POOL_DAEMON_INSTALL_SKIP_PULL:-0}" != "1" ]]; then
  echo "pull ${IMG} (if needed) …" >&2
  "${CLI}" pull "${IMG}"
else
  echo "skip pull (CLAW_POOL_DAEMON_INSTALL_SKIP_PULL=1); using local ${IMG}" >&2
fi
TMP="$(mktemp)"
trap 'rm -f "${TMP}"' EXIT
"${CLI}" run --rm --entrypoint cat "${IMG}" /usr/local/bin/claw-pool-daemon >"${TMP}"
install -m 0755 "${TMP}" "${OUT}"
echo "installed ${OUT}" >&2

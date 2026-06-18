#!/usr/bin/env bash
# Install claw-vscode into Podman OVS + restart container. Author: kejiqing
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
CONTAINER="${CLAW_OVS_CONTAINER:-claw-openvscode-server}"
PORT="${CLAW_OVS_HOST_PORT:-13000}"

"${ROOT_DIR}/deploy/stack/lib/install-claw-vscode-container.sh"

echo "==> restart ${CONTAINER}"
podman restart "${CONTAINER}" >/dev/null

for i in $(seq 1 45); do
  code=$(curl -sS -o /dev/null -w '%{http_code}' "http://127.0.0.1:${PORT}/ovs/" 2>/dev/null || echo 000)
  if [[ "${code}" == "200" ]]; then
    echo "OK: http://127.0.0.1:${PORT}/ovs/ (${code})"
    "${ROOT_DIR}/deploy/stack/lib/verify-ovs-claw-e2e.sh" || exit 1
    echo "Chat → @claw ping"
    exit 0
  fi
  sleep 1
done

echo "ovs-claw-restart: timeout waiting for :${PORT}/ovs/" >&2
exit 1

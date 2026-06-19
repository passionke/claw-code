#!/usr/bin/env bash
# Verify ovs-chat-demo extension in running OVS container. Author: kejiqing
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
# shellcheck source=/dev/null
source "${ROOT_DIR}/deploy/stack/lib/env-profile.sh" 2>/dev/null || true

CONTAINER="${CLAW_OVS_CONTAINER:-claw-openvscode-server}"
EXT_DIR="/opt/claw-extensions"
SD="/opt/claw-ovs/server-data"
OVS_BIN="/home/.openvscode-server/bin/openvscode-server"

fail() { echo "verify-ovs-chat-demo: $*" >&2; exit 1; }

podman container exists "${CONTAINER}" >/dev/null 2>&1 || fail "container ${CONTAINER} not running"

echo "==> list-extensions"
podman exec "${CONTAINER}" "${OVS_BIN}" \
  --list-extensions --extensions-dir="${EXT_DIR}" --server-data-dir="${SD}" \
  | grep -q '^claw\.ovs-chat-demo$' || fail "claw.ovs-chat-demo not installed"

echo "==> extension files"
podman exec "${CONTAINER}" test -f "${EXT_DIR}/claw.ovs-chat-demo-0.2.0/extension.js" \
  || fail "missing extension.js"

echo "==> HTTP /ovs/"
code="$(curl -sS -o /dev/null -w '%{http_code}' http://127.0.0.1:13000/ovs/ 2>/dev/null || echo 000)"
[[ "${code}" == "200" || "${code}" == "302" ]] || fail "OVS HTTP ${code} (expected 200/302)"

echo "OK: ovs-chat-demo installed; OVS HTTP ${code}"
echo "Manual: open http://127.0.0.1:13000/ovs/?folder=/home/workspace/proj_2/home"
echo "        Chat → @demo hello → expect: demo ok"

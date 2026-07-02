#!/usr/bin/env bash
# Smoke-check claw-vscode + claw-code backend for OVS local dev. Author: kejiqing
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
# shellcheck source=/dev/null
[[ -f "${ROOT_DIR}/.env" ]] && source "${ROOT_DIR}/.env"
OVS_ROOT="${CLAW_OVS_ROOT:-${HOME}/work/openvscode-server}"
PORT="${CLAW_OVS_HOST_PORT:-13000}"
GATEWAY_PORT="${GATEWAY_HOST_PORT:-18088}"
PLAYGROUND_PORT="${GATEWAY_PLAYGROUND_HOST_PORT:-18765}"
CONTAINER="${CLAW_OVS_CONTAINER:-claw-openvscode-server}"

fail() { echo "verify-ovs-claw-integration: $*" >&2; exit 1; }

http_code() {
  curl -sS -o /dev/null -w '%{http_code}' "$1" 2>/dev/null || echo "000"
}

echo "==> [1/5] gateway healthz"
code="$(http_code "http://127.0.0.1:${GATEWAY_PORT}/healthz")"
[[ "${code}" == "200" ]] || fail "gateway :${GATEWAY_PORT} HTTP ${code} (run: ./deploy/stack/gateway.sh quick)"
curl -sS "http://127.0.0.1:${GATEWAY_PORT}/healthz" | grep -q '"ok":true' || fail "healthz not ok"

echo "==> [2/5] playground HTTP"
code="$(http_code "http://127.0.0.1:${PLAYGROUND_PORT}/")"
[[ "${code}" == "200" || "${code}" == "302" ]] || fail "playground :${PLAYGROUND_PORT} HTTP ${code}"

echo "==> [3/5] OVS HTTP (compose :${PORT}/ovs/)"
code="$(http_code "http://127.0.0.1:${PORT}/ovs/")"
[[ "${code}" == "200" ]] || fail "OVS :${PORT}/ovs/ HTTP ${code} (run: gateway.sh up && ovs-claw-restart.sh)"
echo "    http://127.0.0.1:${PORT}/ovs/ → ${code}"

echo "==> [4/5] claw.claw-vscode in container"
found=0
if podman container exists "${CONTAINER}" >/dev/null 2>&1; then
  podman exec "${CONTAINER}" /home/.openvscode-server/bin/openvscode-server \
    --list-extensions --extensions-dir=/opt/claw-extensions \
    --server-data-dir=/opt/claw-ovs/server-data 2>/dev/null | grep -q '^claw\.claw-vscode$' && found=1
fi
[[ "${found}" == "1" ]] || fail "claw.claw-vscode not installed (run: ./deploy/stack/lib/ovs-claw-restart.sh)"

echo "==> [5/5] VSIX source present"
[[ -f "${ROOT_DIR}/extensions/claw-vscode/extension.js" ]] || fail "missing extensions/claw-vscode"

echo "OK: backend + extension ready"
echo "Manual E2E:"
echo "  1. Login http://127.0.0.1:${PLAYGROUND_PORT}/admin/login"
echo "  2. Open http://127.0.0.1:${PLAYGROUND_PORT}/ovs/?projId=2"
echo "     or http://127.0.0.1:${PORT}/ovs/?folder=/home/workspace/proj_2/home"
echo "  3. Chat → @claw ping → check Output channel Claw"

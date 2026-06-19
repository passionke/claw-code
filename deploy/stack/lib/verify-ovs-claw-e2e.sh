#!/usr/bin/env bash
# E2E: OVS container → gateway agent WS → claw reply. Author: kejiqing
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
# shellcheck source=/dev/null
[[ -f "${ROOT_DIR}/.env" ]] && source "${ROOT_DIR}/.env"

CONTAINER="${CLAW_OVS_CONTAINER:-claw-openvscode-server}"
GATEWAY_PORT="${GATEWAY_HOST_PORT:-8088}"
PROJ_ID="${CLAW_OVS_E2E_PROJ_ID:-1}"
SESSION_ID="ovs-${PROJ_ID}"
PROMPT="${CLAW_OVS_E2E_PROMPT:-ping}"
TIMEOUT_SEC="${CLAW_OVS_E2E_TIMEOUT_SEC:-90}"

fail() { echo "verify-ovs-claw-e2e: $*" >&2; exit 1; }

podman container exists "${CONTAINER}" >/dev/null 2>&1 || fail "container ${CONTAINER} not running"
curl -sS "http://127.0.0.1:${GATEWAY_PORT}/healthz" | grep -q '"ok":true' || fail "gateway :${GATEWAY_PORT} not healthy"

echo "==> agent WS from ${CONTAINER} (projId=${PROJ_ID} session=${SESSION_ID} prompt=${PROMPT})"
out="$(podman exec "${CONTAINER}" /home/.openvscode-server/node -e "
const WS = globalThis.WebSocket;
const url = 'ws://gateway-rs:8080/v1/sessions/${SESSION_ID}/agent/ws?projId=${PROJ_ID}';
const ws = new WS(url);
let got = false;
let err = '';
ws.onopen = () => {
  ws.send(JSON.stringify({type:'spawn'}));
  ws.send(JSON.stringify({type:'prompt',text:'${PROMPT}\\n'}));
};
ws.onmessage = (e) => {
  got = true;
  try {
    const m = JSON.parse(e.data);
    if (m.type === 'error') { err = m.message || 'agent error'; ws.close(); }
    if (m.type === 'cdp' && m.event && m.event.ev === 'status' && m.event.phase === 'done') ws.close();
  } catch {}
};
ws.onerror = () => { if (!got) err = 'websocket error'; };
setTimeout(() => { if (!got && !err) err = 'timeout'; ws.close(); }, ${TIMEOUT_SEC}000);
ws.onclose = () => {
  if (err) { console.log('FAIL:' + err); process.exit(1); }
  if (!got) { console.log('FAIL:no response'); process.exit(2); }
  console.log('OK projId=${PROJ_ID} session=${SESSION_ID}');
  process.exit(0);
};
" 2>&1)" || {
  echo "${out}"
  echo "hint: ./deploy/stack/gateway.sh pool-reset && ./deploy/stack/gateway.sh up" >&2
  exit 1
}

echo "${out}"
echo "verify-ovs-claw-e2e: OK"

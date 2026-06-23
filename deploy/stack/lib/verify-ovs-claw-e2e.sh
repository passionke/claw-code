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

run_agent_ws_host() {
  GATEWAY_PORT="${GATEWAY_PORT}" SESSION_ID="${SESSION_ID}" PROJ_ID="${PROJ_ID}" \
    PROMPT="${PROMPT}" TIMEOUT_SEC="${TIMEOUT_SEC}" python3 - <<'PY'
import json, os, sys, time
try:
    import websocket
except ImportError:
    import subprocess
    subprocess.check_call([sys.executable, "-m", "pip", "install", "-q", "websocket-client"])
    import websocket

port = os.environ["GATEWAY_PORT"]
sid = os.environ["SESSION_ID"]
pid = os.environ["PROJ_ID"]
prompt = os.environ.get("PROMPT", "ping")
timeout_sec = int(os.environ.get("TIMEOUT_SEC", "90"))
url = f"ws://127.0.0.1:{port}/v1/sessions/{sid}/agent/ws?projId={pid}"
got = False
err = ""

def on_message(ws, message):
    global got, err
    got = True
    try:
        m = json.loads(message)
        if m.get("type") == "error":
            err = m.get("message") or "agent error"
            ws.close()
        elif os.environ.get("CLAW_OVS_E2E_FAST", "0") in ("1", "true", "yes"):
            ws.close()
        elif m.get("type") == "cdp" and (m.get("event") or {}).get("phase") == "done":
            ws.close()
    except Exception:
        if os.environ.get("CLAW_OVS_E2E_FAST", "0") in ("1", "true", "yes"):
            ws.close()

def on_error(ws, error):
    global err
    if not got:
        err = str(error)

def on_open(ws):
    ws.send(json.dumps({"type": "prompt", "text": prompt + "\n"}))

ws = websocket.WebSocketApp(url, on_open=on_open, on_message=on_message, on_error=on_error)
deadline = time.time() + timeout_sec
while time.time() < deadline and ws.sock is None:
    time.sleep(0.05)
ws.run_forever(ping_interval=20, ping_timeout=10)
if err:
    print("FAIL:" + err)
    sys.exit(1)
if not got:
    print("FAIL:no response")
    sys.exit(2)
print(f"OK projId={pid} session={sid}")
PY
}

run_agent_ws_multi_host() {
  GATEWAY_PORT="${GATEWAY_PORT}" SESSION_ID="${SESSION_ID}" PROJ_ID="${PROJ_ID}" \
    CHAT_SESSION_ID="${CHAT_SESSION_ID:-e2e-multi}" TIMEOUT_SEC="${TIMEOUT_SEC}" python3 - <<'PY'
import json, os, sys, time
try:
    import websocket
except ImportError:
    import subprocess
    subprocess.check_call([sys.executable, "-m", "pip", "install", "-q", "websocket-client"])
    import websocket

port = os.environ["GATEWAY_PORT"]
sid = os.environ["SESSION_ID"]
pid = os.environ["PROJ_ID"]
chat = os.environ.get("CHAT_SESSION_ID", "e2e-multi")
timeout_sec = int(os.environ.get("TIMEOUT_SEC", "90"))
url = f"ws://127.0.0.1:{port}/v1/sessions/{sid}/agent/ws?projId={pid}&chatSessionId={chat}"
prompts = [
    "Remember the secret codeword ZEBRA42 for this chat only.\n",
    "What secret codeword did I ask you to remember? Reply with the codeword only.\n",
]
replies = []
err = ""

def run_prompt(prompt):
    global err
    got = False
    text_parts = []

    def on_message(ws, message):
        nonlocal got, err
        got = True
        try:
            m = json.loads(message)
            if m.get("type") == "error":
                err = m.get("message") or "agent error"
                ws.close()
            elif m.get("type") == "cdp":
                ev = m.get("event") or {}
                if ev.get("ev") == "content.delta" and ev.get("text"):
                    text_parts.append(ev["text"])
                if ev.get("ev") == "status" and ev.get("phase") in ("done", "failed"):
                    ws.close()
        except Exception:
            pass

    def on_open(ws):
        ws.send(json.dumps({"type": "prompt", "text": prompt}))

    ws = websocket.WebSocketApp(url, on_open=on_open, on_message=on_message)
    ws.run_forever(ping_interval=20, ping_timeout=10)
    if err:
        raise RuntimeError(err)
    if not got:
        raise RuntimeError("no response")
    replies.append("".join(text_parts))

for p in prompts:
    run_prompt(p)
    time.sleep(0.5)

if "ZEBRA42" not in replies[-1].upper():
    print("FAIL:multi-turn context missing ZEBRA42 in reply:", replies[-1][:200])
    sys.exit(3)

nas = os.environ.get("CLAW_NAS_HOST_MOUNT", "").strip()
if nas:
    import pathlib
    record = f"ovs-chat-{pid}-{chat}"
    jsonl = pathlib.Path(nas) / f"proj_{pid}/sessions/{record}/interactive-session.jsonl"
    if not jsonl.is_file():
        print(f"FAIL:missing consolidated jsonl {jsonl}")
        sys.exit(4)
    lines = [ln for ln in jsonl.read_text(encoding="utf-8").splitlines() if ln.strip()]
    if len(lines) < 3:
        print(f"FAIL:jsonl too short ({len(lines)} lines) at {jsonl}")
        sys.exit(5)
    body = jsonl.read_text(encoding="utf-8")
    if "ZEBRA42" not in body:
        print(f"FAIL:jsonl missing ZEBRA42 at {jsonl}")
        sys.exit(6)

print(f"OK multi-turn projId={pid} chat={chat}")
PY
}

if [[ "${CLAW_OVS_E2E_SKIP_CONTAINER:-0}" == "1" ]]; then
  curl -sS "http://127.0.0.1:${GATEWAY_PORT}/healthz" | grep -q '"ok":true' || fail "gateway :${GATEWAY_PORT} not healthy"
  echo "==> agent WS from host (projId=${PROJ_ID} session=${SESSION_ID} prompt=${PROMPT})"
  out="$(run_agent_ws_host 2>&1)" || { echo "${out}"; exit 1; }
  echo "${out}"
  if [[ "${CLAW_OVS_E2E_MULTI_TURN:-0}" == "1" ]]; then
    echo "==> multi-turn agent WS (chatSessionId=e2e-multi)"
    out="$(run_agent_ws_multi_host 2>&1)" || { echo "${out}"; exit 1; }
    echo "${out}"
  fi
  echo "verify-ovs-claw-e2e: OK"
  exit 0
fi

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

#!/usr/bin/env bash
# One-shot live report route audit: solve → poll → gateway SSE + playground proxy SSE → log hints.
# Author: kejiqing
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# shellcheck disable=SC1090
source "${ROOT}/.env"

export CLAW_LIVE_REPORT_ROUTE_AUDIT=1
export CLAW_REPORT_SSE_TIMING=1

PORT="${GATEWAY_HOST_PORT:-18088}"
PG_PORT="${GATEWAY_PLAYGROUND_HOST_PORT:-18765}"
BASE="http://127.0.0.1:${PORT}"
PLAY="http://127.0.0.1:${PG_PORT}"
PROMPT="${LIVE_REPORT_E2E_PROMPT:-用三句话介绍你自己}"
LOG_DIR="${ROOT}/deploy/stack/claw-logs"

echo "==> ensure stack (gateway + playground); set CLAW_LIVE_REPORT_ROUTE_AUDIT=1 in .env for container logs"
echo "    run: ./deploy/stack/gateway.sh pack-deploy   # after Rust changes"

echo "==> POST solve_async"
TASK_JSON="$(curl -fsS -X POST "${BASE}/v1/solve_async" \
  -H "Content-Type: application/json" \
  -d "{\"dsId\":1,\"userPrompt\":$(python3 -c 'import json,sys;print(json.dumps(sys.argv[1]))' "${PROMPT}")}")"
echo "${TASK_JSON}"
SESSION_ID="$(printf '%s' "${TASK_JSON}" | python3 -c 'import json,sys;print(json.load(sys.stdin)["sessionId"])')"
TURN_ID="$(printf '%s' "${TASK_JSON}" | python3 -c 'import json,sys;print(json.load(sys.stdin)["turnId"])')"

export BASE PLAY SESSION_ID TURN_ID
python3 <<'PY'
import json
import os
import sys
import threading
import time
import urllib.parse
import urllib.request
from http.client import HTTPConnection

base = os.environ["BASE"]
play = os.environ["PLAY"]
session_id = os.environ["SESSION_ID"]
turn_id = os.environ["TURN_ID"]

q = urllib.parse.urlencode(
    {"sessionId": session_id, "turnId": turn_id, "dsId": "1", "stream": "true"}
)
gw_path = f"/v1/biz_advice_report?{q}"
proxy_path = "/__proxy_sse__?" + urllib.parse.urlencode(
    {"target": base + gw_path}
)


def count_sse(url: str, label: str) -> dict:
    pu = urllib.parse.urlparse(url)
    conn = HTTPConnection(pu.hostname, pu.port, timeout=600)
    t0 = time.perf_counter()
    conn.request("GET", pu.path + ("?" + pu.query if pu.query else ""), headers={"Accept": "text/event-stream"})
    resp = conn.getresponse()
    if resp.status != 200:
        return {"label": label, "error": f"HTTP {resp.status}", "deltas": 0}
    buf = b""
    deltas = 0
    first_ms = None
    while True:
        chunk = resp.read(1024)
        if not chunk:
            break
        now = time.perf_counter()
        if first_ms is None:
            first_ms = (now - t0) * 1000
        buf += chunk
        while b"\n\n" in buf:
            frame, buf = buf.split(b"\n\n", 1)
            if b"biz.report.delta" in frame:
                deltas += 1
    return {
        "label": label,
        "deltas": deltas,
        "first_byte_ms": first_ms,
        "wall_ms": (time.perf_counter() - t0) * 1000,
    }


def tasks():
    with urllib.request.urlopen(f"{base}/v1/tasks/{session_id}", timeout=30) as r:
        return json.loads(r.read())


print("==> poll until hasReport")
sse_started = False
result = {}

def run_gw():
    result["gw"] = count_sse(base + gw_path, "gateway_direct")


def run_px():
    result["px"] = count_sse(play + proxy_path, "playground_proxy")


for i in range(1, 301):
    t = tasks()
    st = t.get("status", "")
    has = bool(t.get("hasReport"))
    print(f"  [{i}] status={st} hasReport={has}")
    if has and not sse_started:
        sse_started = True
        threading.Thread(target=run_gw, daemon=True).start()
        threading.Thread(target=run_px, daemon=True).start()
    if st in ("succeeded", "failed", "cancelled"):
        break
    time.sleep(0.5)

time.sleep(2)
for key in ("gw", "px"):
    if key in result:
        print("==>", result[key])

print()
print("==> grep gateway logs (route + proxy_stream_end)")
print(f"    turnId={turn_id}")
PY

if [[ -d "${LOG_DIR}" ]]; then
  rg -n "${TURN_ID}|claw_gateway_live_report|gateway_proxy_stream_end|worker_report_route" "${LOG_DIR}" 2>/dev/null | tail -40 || true
else
  echo "    (no ${LOG_DIR}; check podman logs claw-gateway-rs)"
  podman logs --tail 80 claw-gateway-rs 2>/dev/null | rg "${TURN_ID}|claw_gateway_live_report|gateway_proxy_stream_end" || true
fi

echo ""
echo "==> playground stderr (proxy_sse_end) — run: podman logs claw-gateway-playground 2>&1 | rg claw_proxy_sse_end"

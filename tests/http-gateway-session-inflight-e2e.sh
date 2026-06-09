#!/usr/bin/env bash
# E2E: same session turn1 running → turn2 POST /v1/solve_async returns 409 inflight. Author: kejiqing
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
LIB_DIR="${REPO_ROOT}/deploy/stack/lib"
PODMAN_DIR="${REPO_ROOT}/deploy/stack"
# shellcheck disable=SC1091
source "${LIB_DIR}/pool-health.sh"

GATEWAY_PORT="${GATEWAY_HOST_PORT:-18088}"
BASE="http://127.0.0.1:${GATEWAY_PORT}"
DS_ID="${DS_ID:-1}"

claw_assert_gateway_pool_http_reachable "${PODMAN_DIR}"

export GATEWAY_PORT DS_ID
python3 <<'PY'
import json, os, threading, time, urllib.error, urllib.request

port = int(os.environ["GATEWAY_PORT"])
base = f"http://127.0.0.1:{port}"
ds = int(os.environ["DS_ID"])

cfg = json.load(urllib.request.urlopen(f"{base}/v1/project/config/{ds}", timeout=15))
extra = {"tenant_code":"GPOS","solution_code":"restaurant","biz_type":"BOSS_REPORT","_claw_client_origin":"gateway-admin"}
for f in (cfg.get("extraSessionFieldsJson") or []):
    if isinstance(f, str) and f.strip():
        extra[f.strip()] = ""

def post(prompt, sid=None):
    body = {"projId": ds, "userPrompt": prompt, "extraSession": extra, "timeoutSeconds": 240}
    if sid:
        body["sessionId"] = sid
    req = urllib.request.Request(
        f"{base}/v1/solve_async",
        data=json.dumps(body).encode(),
        method="POST",
        headers={"Content-Type": "application/json"},
    )
    try:
        with urllib.request.urlopen(req, timeout=30) as resp:
            return resp.status, json.load(resp)
    except urllib.error.HTTPError as e:
        return e.code, json.loads(e.read().decode() or "{}")

# Long-running turn1 so turn2 hits inflight gate while turn1 is still running.
slow = (
    "Use bash only: sleep 45; echo SLOW_DONE"
)
code1, r1 = post(slow)
if code1 != 200:
    raise SystemExit(f"round1 enqueue failed: HTTP {code1} {r1}")
sid = r1["sessionId"]
print(f"[e2e] turn1 enqueued session={sid} task={r1['taskId']}")

time.sleep(3)
code2, r2 = post("ping", sid)
print(f"[e2e] turn2 while turn1 running: HTTP {code2}")
if code2 != 409:
    raise SystemExit(f"FAIL: expected HTTP 409 inflight, got {code2}: {r2}")
detail = json.dumps(r2)
# Async path: gateway in-memory guard ("active async task"); sync/PG path: "inflight".
if "inflight" not in detail and "active async task" not in detail:
    raise SystemExit(f"FAIL: expected inflight or active async task in body, got {detail}")

# Wait for turn1 to finish so we do not leave a long-running task.
task1 = r1["taskId"]
for _ in range(90):
    rec = json.load(urllib.request.urlopen(f"{base}/v1/tasks/{task1}", timeout=15))
    if rec["status"] in ("succeeded", "failed", "cancelled"):
        print(f"[e2e] turn1 finished status={rec['status']}")
        break
    time.sleep(2)
else:
    raise SystemExit("turn1 poll timeout")

print("OK — http-gateway-session-inflight-e2e")
PY

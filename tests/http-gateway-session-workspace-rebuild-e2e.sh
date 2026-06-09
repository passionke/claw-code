#!/usr/bin/env bash
# E2E: pool v1 workspace files PG round-trip — write file, kill worker, delete ② cache,续聊 ls. Author: kejiqing
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
LIB_DIR="${REPO_ROOT}/deploy/stack/lib"
PODMAN_DIR="${REPO_ROOT}/deploy/stack"
# shellcheck disable=SC1091
source "${LIB_DIR}/pool-health.sh"

GATEWAY_PORT="${GATEWAY_HOST_PORT:-18088}"
BASE="http://127.0.0.1:${GATEWAY_PORT}"
DS_ID="${DS_ID:-1}"
PGURL="${CLAW_GATEWAY_DATABASE_URL:-postgres://claw_gateway:clawGw9Dev_Pg@127.0.0.1:5433/claw_gateway}"
WORK_HOST="${CLAW_WORK_ROOT_HOST:-${REPO_ROOT}/deploy/stack/claw-workspace}"

claw_assert_gateway_pool_http_reachable "${PODMAN_DIR}"

MARK="wsrebuild_$(date +%s)_$$"
export GATEWAY_PORT DS_ID MARK PGURL WORK_HOST
python3 <<PY
import json, os, shutil, subprocess, time, urllib.request

port = int(os.environ["GATEWAY_PORT"])
base = f"http://127.0.0.1:{port}"
ds = int(os.environ["DS_ID"])
mark = os.environ["MARK"]
pg = os.environ["PGURL"]
work_host = os.environ["WORK_HOST"]

cfg = json.load(urllib.request.urlopen(f"{base}/v1/project/config/{ds}", timeout=15))
extra = {"tenant_code":"GPOS","solution_code":"restaurant","biz_type":"BOSS_REPORT","_claw_client_origin":"gateway-admin"}
for f in (cfg.get("extraSessionFieldsJson") or []):
    if isinstance(f, str) and f.strip():
        extra[f.strip()] = ""

def post(prompt, sid=None):
    body = {"projId": ds, "userPrompt": prompt, "extraSession": extra, "timeoutSeconds": 240}
    if sid:
        body["sessionId"] = sid
    req = urllib.request.Request(f"{base}/v1/solve_async", data=json.dumps(body).encode(), method="POST", headers={"Content-Type":"application/json"})
    return json.load(urllib.request.urlopen(req, timeout=30))

def poll(task):
    for _ in range(150):
        r = json.load(urllib.request.urlopen(f"{base}/v1/tasks/{task}", timeout=15))
        if r["status"] in ("succeeded","failed","cancelled"):
            return r
        time.sleep(2)
    raise SystemExit("task timeout")

def psql(q):
    return subprocess.check_output(["psql", pg, "-t", "-A", "-c", q], text=True).strip()

fname = f"claw_{mark}.txt"
p1 = f"Use bash only: printf '%s\\n' '{mark}' > {fname} && cat {fname}"
print("[e2e] round1 write", fname)
r1 = post(p1)
sid, tid1 = r1["sessionId"], r1["turnId"]
rec1 = poll(r1["taskId"])
if rec1["status"] != "succeeded":
    raise SystemExit(f"round1 failed: {rec1.get('error')}")

art = psql(f"SELECT COUNT(*) FROM gateway_session_artifacts WHERE session_id='{sid}' AND kind='workspace_tar_gz'")
ready = psql(f"SELECT artifacts_ready FROM gateway_turns WHERE turn_id='{tid1}'")
home_rel = psql(f"SELECT session_home FROM gateway_sessions WHERE session_id='{sid}'")
host = f"{work_host}/{home_rel}"
print(f"[e2e] artifacts={art} artifacts_ready={ready} host={host}")

if art != "1":
    raise SystemExit(f"FAIL: expected 1 artifact row for {fname}, got {art}")
if ready != "t":
    raise SystemExit(f"FAIL: artifacts_ready not true after round1")

if os.environ.get("COLD_KILL_WORKERS") == "1":
    workers = subprocess.check_output(["podman", "ps", "-a", "--format", "{{.Names}}"], text=True).splitlines()
    to_rm = [n for n in workers if n.startswith("claw-worker-")]
    if to_rm:
        subprocess.run(["podman", "rm", "-f", *to_rm], capture_output=True)
    print("[e2e] killed worker containers (COLD_KILL_WORKERS=1)")
if os.path.isdir(host):
    shutil.rmtree(host)
print("[e2e] deleted gateway cache dir (②); worker tmpfs wiped on next materialize_in")

# pool warm: wait for health
for _ in range(30):
    try:
        urllib.request.urlopen(f"http://127.0.0.1:9944/healthz/live-report", timeout=3)
        break
    except Exception:
        time.sleep(2)

p2 = f"Use bash only: test -f {fname} && cat {fname} || echo FILE_MISSING"
print("[e2e] round2 cold read", fname)
r2 = post(p2, sid)
rec2 = poll(r2["taskId"])
if rec2["status"] != "succeeded":
    raise SystemExit(f"round2 failed: {rec2.get('error')}")
out = (rec2.get("result") or {}).get("outputText") or ""
try:
    j = json.loads(out)
    out = j.get("message", out)
except Exception:
    pass
print("[e2e] round2 output:", out[:400])
if "FILE_MISSING" in out:
    raise SystemExit("FAIL: workspace file not restored from PG after cold start")
if mark not in out:
    raise SystemExit(f"FAIL: expected marker {mark} in round2 output, got: {out[:400]!r}")

print("OK — http-gateway-session-workspace-rebuild-e2e")
PY

#!/usr/bin/env bash
# Live report audit E2E: solve → hasReport 即开 SSE → 对账 PG / SSE / gateway 日志。Author: kejiqing
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# shellcheck disable=SC1090
source "${ROOT}/.env"

PORT="${GATEWAY_HOST_PORT:-18088}"
BASE="http://127.0.0.1:${PORT}"
PGURL="postgres://claw_gateway:clawGw9Dev_Pg@127.0.0.1:${CLAW_GATEWAY_PG_HOST_PORT:-5433}/claw_gateway"
PROMPT="${LIVE_REPORT_E2E_PROMPT:-用一句话说你好，不要查库}"

echo "==> POST solve_async"
TASK_JSON="$(curl -fsS -X POST "${BASE}/v1/solve_async" \
  -H "Content-Type: application/json" \
  -d "{\"dsId\":1,\"userPrompt\":$(python3 -c 'import json,sys;print(json.dumps(sys.argv[1]))' "${PROMPT}")}")"
echo "${TASK_JSON}"
TASK_ID="$(printf '%s' "${TASK_JSON}" | python3 -c 'import json,sys;print(json.load(sys.stdin)["taskId"])')"
TURN_ID="$(printf '%s' "${TASK_JSON}" | python3 -c 'import json,sys;print(json.load(sys.stdin)["turnId"])')"
SESSION_ID="${TASK_ID}"
OUT="/tmp/claw_live_sse_${TURN_ID}.txt"

export BASE SESSION_ID TURN_ID OUT PGURL
python3 <<'PY'
import json, os, subprocess, sys, threading, time, urllib.request

base = os.environ["BASE"]
session_id = os.environ["SESSION_ID"]
turn_id = os.environ["TURN_ID"]
out_path = os.environ["OUT"]
pgurl = os.environ["PGURL"]


def tasks_get():
    url = f"{base}/v1/tasks/{session_id}"
    with urllib.request.urlopen(url, timeout=30) as r:
        return json.loads(r.read())


def pg_chunks():
    q = f"SELECT COUNT(*)::int FROM gateway_turn_live_chunks WHERE turn_id='{turn_id}'"
    r = subprocess.run(
        ["psql", pgurl, "-t", "-A", "-c", q],
        capture_output=True,
        text=True,
        check=True,
    )
    return int(r.stdout.strip() or "0")


def sse_reader():
    url = (
        f"{base}/v1/biz_advice_report?sessionId={session_id}"
        f"&turnId={turn_id}&dsId=1&stream=true"
    )
    deltas = 0
    bytes_out = 0
    with urllib.request.urlopen(url, timeout=600) as r, open(out_path, "wb") as f:
        buf = b""
        while True:
            chunk = r.read(4096)
            if not chunk:
                break
            f.write(chunk)
            bytes_out += len(chunk)
            buf += chunk
            while b"\n\n" in buf:
                frame, buf = buf.split(b"\n\n", 1)
                if b"event: biz.report.delta" in frame:
                    deltas += 1
    return deltas, bytes_out


sse_started = False
sse_result = {"deltas": 0, "bytes": 0, "err": None}
sse_thread = None


def run_sse():
    try:
        d, b = sse_reader()
        sse_result["deltas"] = d
        sse_result["bytes"] = b
    except Exception as e:
        sse_result["err"] = str(e)


print("==> poll tasks; open SSE on first hasReport (during running)")
for i in range(1, 301):
    t = tasks_get()
    st = t.get("status", "")
    has = bool(t.get("hasReport"))
    rt = t.get("reportTime") or ""
    pg_n = pg_chunks() if has else 0
    print(f"  [{i}] status={st} hasReport={has} reportTime={rt} pg_chunks={pg_n}")
    if has and not sse_started:
        sse_started = True
        sse_thread = threading.Thread(target=run_sse, daemon=True)
        sse_thread.start()
        print(f"      SSE background → {out_path}")
    if st in ("succeeded", "failed", "cancelled"):
        break
    time.sleep(2)

if not sse_started:
    print("FAIL: hasReport never true", file=sys.stderr)
    sys.exit(1)

if sse_thread:
    sse_thread.join(timeout=180)
pg_final = pg_chunks()
print(f"==> PG chunks final={pg_final}")
print(f"==> SSE file={out_path} deltas={sse_result['deltas']} bytes={sse_result['bytes']} err={sse_result['err']}")

if sse_result["err"]:
    print(f"WARN: SSE reader: {sse_result['err']}", file=sys.stderr)

print("==> gateway audit (tail 40 for turn)")
subprocess.run(
    [
        "bash",
        "-lc",
        f"podman logs claw-gateway-rs 2>&1 | rg '{turn_id}' | rg 'sse_tail_emit|sse_loop_wake|pg_notify_received' | tail -40",
    ],
    check=False,
)

ratio = (sse_result["deltas"] / pg_final) if pg_final else 0
print(f"==> done taskId={session_id} turnId={turn_id} delta/pg_ratio={ratio:.3f}")
if pg_final and sse_result["deltas"] and ratio < 0.5:
    print("WARN: SSE deltas << PG chunks during live read (re-run curl after succeeded)", file=sys.stderr)
    sys.exit(2)
PY

echo "==> post-hoc curl SSE event count (full catch-up)"
EVENT_N="$(curl -fsS -N "${BASE}/v1/biz_advice_report?sessionId=${SESSION_ID}&turnId=${TURN_ID}&dsId=1&stream=true" 2>/dev/null | rg -c '^event:' || true)"
echo "    curl event lines=${EVENT_N} pg_chunks=$(psql "${PGURL}" -t -A -c "SELECT COUNT(*) FROM gateway_turn_live_chunks WHERE turn_id='${TURN_ID}'")"

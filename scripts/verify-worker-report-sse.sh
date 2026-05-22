#!/usr/bin/env bash
# Worker report SSE: unit tests + optional live rhythm check (solve → worker :29000).
# Author: kejiqing
#
#   ./scripts/verify-worker-report-sse.sh              # cargo test only
#   ./scripts/verify-worker-report-sse.sh --e2e        # solve_async + worker SSE timing (fail on burst)
#   ./scripts/verify-worker-report-sse.sh T_xxx 29000  # manual turn during running solve
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
MODE="${1:-}"

echo "== worker report_sse_server unit tests =="
(cd "$ROOT/rust" && cargo test -p gateway-solve-turn report_sse_server -- --nocapture)

if [[ "$MODE" == "--e2e" ]]; then
  :
elif [[ -n "$MODE" ]]; then
  TURN_ID="$MODE"
  HOST_PORT="${2:-29000}"
  STATUS_URL="http://127.0.0.1:${HOST_PORT}/v1/turns/${TURN_ID}/report/status"
  REPORT_URL="http://127.0.0.1:${HOST_PORT}/v1/turns/${TURN_ID}/report"
  echo ""
  echo "== live worker (host port ${HOST_PORT}) =="
  curl -sfS -m 3 "$STATUS_URL" | python3 -m json.tool
  TMP="$(mktemp)"
  trap 'rm -f "$TMP"' EXIT
  curl -sS -N -m "${MAX_SSE_SEC:-8}" -H "Accept: text/event-stream" "$REPORT_URL" >"$TMP" || true
  DELTAS="$(rg -c 'event: biz.report.delta' "$TMP" 2>/dev/null || echo 0)"
  echo "delta_frames=${DELTAS} bytes=$(wc -c <"$TMP" | tr -d ' ')"
  exit 0
else
  echo ""
  echo "Unit tests OK. Full rhythm check:"
  echo "  ./deploy/stack/gateway.sh pack-deploy   # after Rust changes"
  echo "  ./scripts/verify-worker-report-sse.sh --e2e"
  exit 0
fi

# shellcheck disable=SC1090
source "${ROOT}/.env"

PORT="${GATEWAY_HOST_PORT:-18088}"
BASE="http://127.0.0.1:${PORT}"
WORKER_PORT="${WORKER_REPORT_E2E_PORT:-29000}"
PROMPT="${LIVE_REPORT_E2E_PROMPT:-用三句话介绍你自己}"
MAX_BURST_GAP_MS="${MAX_BURST_GAP_MS:-2}"
MAX_BURST_RATIO="${MAX_BURST_RATIO:-0.65}"

echo ""
echo "== E2E: solve_async → poll hasReport → worker SSE rhythm (port ${WORKER_PORT}) =="
echo "    gateway: ${BASE}"

echo "==> POST solve_async"
TASK_JSON="$(curl -fsS -X POST "${BASE}/v1/solve_async" \
  -H "Content-Type: application/json" \
  -d "{\"dsId\":1,\"userPrompt\":$(python3 -c 'import json,sys;print(json.dumps(sys.argv[1]))' "${PROMPT}")}")"
SESSION_ID="$(printf '%s' "${TASK_JSON}" | python3 -c 'import json,sys;print(json.load(sys.stdin)["sessionId"])')"
TURN_ID="$(printf '%s' "${TASK_JSON}" | python3 -c 'import json,sys;print(json.load(sys.stdin)["turnId"])')"
echo "sessionId=${SESSION_ID} turnId=${TURN_ID}"

export BASE SESSION_ID TURN_ID WORKER_PORT MAX_BURST_GAP_MS MAX_BURST_RATIO
python3 <<'PY'
import json
import os
import sys
import threading
import time
import urllib.request
from http.client import HTTPConnection, IncompleteRead

base = os.environ["BASE"]
session_id = os.environ["SESSION_ID"]
turn_id = os.environ["TURN_ID"]
prefer_port = int(os.environ["WORKER_PORT"])
max_gap = float(os.environ["MAX_BURST_GAP_MS"])
max_ratio = float(os.environ["MAX_BURST_RATIO"])

worker_path = f"/v1/turns/{turn_id}/report"


def tasks():
    with urllib.request.urlopen(f"{base}/v1/tasks/{session_id}", timeout=30) as r:
        return json.loads(r.read())


def worker_status(port: int) -> dict | None:
    try:
        with urllib.request.urlopen(
            f"http://127.0.0.1:{port}/v1/turns/{turn_id}/report/status", timeout=2
        ) as r:
            return json.loads(r.read())
    except OSError:
        return None


def pick_worker_port() -> int:
    for port in (prefer_port, 29000, 29001):
        st = worker_status(port)
        if st and st.get("hasReport"):
            return port
    return prefer_port


def read_worker_sse(port: int, max_sec: float = 120.0) -> dict:
    conn = HTTPConnection("127.0.0.1", port, timeout=max_sec)
    t0 = time.perf_counter()
    try:
        conn.request(
            "GET",
            worker_path,
            headers={"Accept": "text/event-stream"},
        )
        resp = conn.getresponse()
    except OSError as e:
        return {"error": str(e), "deltas": 0, "port": port}
    if resp.status != 200:
        return {"error": f"HTTP {resp.status}", "deltas": 0, "port": port}
    import re

    raw = b""
    deadline = t0 + max_sec
    truncated = False
    while time.perf_counter() < deadline:
        try:
            chunk = resp.read(1024)
        except IncompleteRead as e:
            chunk = e.partial
            truncated = True
        if not chunk:
            break
        raw += chunk
        if truncated:
            break
    text = raw.decode("utf-8", errors="replace")
    delta_t = [int(m.group(1)) for m in re.finditer(r'"t":(\d+)', text)]
    gaps = [float(delta_t[i + 1] - delta_t[i]) for i in range(len(delta_t) - 1)]
    under = sum(1 for g in gaps if g < max_gap)
    ratio = (under / len(gaps)) if gaps else 0.0
    window = 2
    max_in_window = 0
    for i in range(len(delta_t)):
        w_end = delta_t[i] + window
        count = sum(1 for t in delta_t if delta_t[i] <= t <= w_end)
        max_in_window = max(max_in_window, count)
    return {
        "port": port,
        "deltas": len(delta_t),
        "delta_t_list": delta_t,
        "gaps": gaps[:20],
        "under_2ms": under,
        "under_ratio": ratio,
        "max_in_2ms_window": max_in_window,
        "truncated": truncated,
        "wall_ms": (time.perf_counter() - t0) * 1000.0,
    }


sse_box: dict = {}
done = threading.Event()


def worker_reader():
    port = pick_worker_port()
    sse_box["result"] = read_worker_sse(port, 120.0)
    done.set()


print("==> poll until hasReport, start worker SSE in background")
for i in range(1, 301):
    t = tasks()
    st = t.get("status", "")
    has = bool(t.get("hasReport"))
    print(f"  [{i}] status={st} hasReport={has}")
    if has and "thread" not in sse_box:
        port = pick_worker_port()
        print(f"==> worker SSE on port {port} (background)")
        sse_box["thread"] = threading.Thread(target=worker_reader, daemon=True)
        sse_box["thread"].start()
    if st in ("succeeded", "failed", "cancelled"):
        break
    time.sleep(0.3)

done.wait(timeout=30)
sse = sse_box.get("result")
if sse:
    print(json.dumps(sse, indent=2))

if sse is None:
    print("FAIL: never got hasReport or worker SSE", file=sys.stderr)
    sys.exit(1)
if sse.get("error"):
    print("FAIL worker SSE:", sse["error"], file=sys.stderr)
    sys.exit(1)
if sse["deltas"] < 3:
    print("FAIL: too few deltas (model may not have streamed report)", file=sys.stderr)
    sys.exit(1)

gaps = sse["gaps"]
ratio = sse["under_ratio"]
max_win = sse["max_in_2ms_window"]
print(f"==> rhythm: deltas={sse['deltas']} gaps={len(gaps)} under_{max_gap}ms={sse['under_2ms']} ratio={ratio:.2f} max_in_2ms_window={max_win}")

from collections import Counter

same_t_burst = max(Counter(sse.get("delta_t_list") or []).values()) if sse.get("delta_t_list") else 0

failed = False
if len(gaps) >= 5 and ratio > max_ratio:
    print(
        f"FAIL: {ratio:.0%} of server-time gaps < {max_gap}ms (threshold {max_ratio:.0%})",
        file=sys.stderr,
    )
    failed = True
if max_win >= 12:
    print(
        f"FAIL: {max_win} deltas with hub `t` within {2}ms — coalesced burst",
        file=sys.stderr,
    )
    failed = True
if same_t_burst >= 15:
    print(
        f"FAIL: {same_t_burst} deltas share the same hub timestamp `t`",
        file=sys.stderr,
    )
    failed = True
print(f"    server_t: max_same_t={same_t_burst}")
if failed:
    sys.exit(1)
print("OK worker SSE rhythm (E2E)")
PY

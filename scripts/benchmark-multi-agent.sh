#!/usr/bin/env bash
# Multi-agent solve wall-clock benchmark (P50 acceptance: <= 120s). Author: kejiqing
set -euo pipefail

GATEWAY="${GATEWAY:-http://127.0.0.1:8080}"
DS_ID="${DS_ID:-1}"
RUNS="${RUNS:-5}"
PROMPT_FILE="${PROMPT_FILE:-$(dirname "$0")/fixtures/multi-agent-benchmark-prompt.txt}"
ACCEPT_P50_MS="${ACCEPT_P50_MS:-120000}"

usage() {
  cat <<EOF
Usage: $0 [--gateway URL] [--ds-id N] [--runs N] [--prompt-file PATH]

Measures wall-clock from POST /v1/solve_async until task succeeded/failed.
Prints per-run durations and P50. Exit 0 if P50 <= ${ACCEPT_P50_MS}ms.

Env: GATEWAY, DS_ID, RUNS, PROMPT_FILE, ACCEPT_P50_MS
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --gateway) GATEWAY="$2"; shift 2 ;;
    --ds-id) DS_ID="$2"; shift 2 ;;
    --runs) RUNS="$2"; shift 2 ;;
    --prompt-file) PROMPT_FILE="$2"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "Unknown arg: $1" >&2; usage; exit 1 ;;
  esac
done

export GATEWAY DS_ID RUNS PROMPT_FILE ACCEPT_P50_MS
exec python3 - <<'PY'
import json
import os
import sys
import time
import urllib.error
import urllib.request

gateway = os.environ["GATEWAY"].rstrip("/")
ds_id = int(os.environ["DS_ID"])
runs = int(os.environ["RUNS"])
prompt_file = os.environ["PROMPT_FILE"]
accept_p50 = int(os.environ["ACCEPT_P50_MS"])

with open(prompt_file, encoding="utf-8") as f:
    prompt = f.read().strip()
if not prompt:
    print("empty prompt", file=sys.stderr)
    sys.exit(1)


def http(method: str, path: str, body: dict | None = None) -> dict:
    data = None
    headers = {"Content-Type": "application/json"}
    if body is not None:
        data = json.dumps(body).encode("utf-8")
    req = urllib.request.Request(
        f"{gateway}{path}", data=data, headers=headers, method=method
    )
    with urllib.request.urlopen(req, timeout=300) as resp:
        return json.loads(resp.read().decode("utf-8"))


durations: list[int] = []
for i in range(1, runs + 1):
    print(f"=== run {i}/{runs} ===")
    start_ms = int(time.time() * 1000)
    created = http("POST", "/v1/solve_async", {"projId": ds_id, "prompt": prompt})
    task_id = created["taskId"]
    print(f"taskId={task_id}")
    while True:
        task = http("GET", f"/v1/tasks/{task_id}")
        status = task.get("status", "")
        if status in ("succeeded", "failed", "cancelled"):
            elapsed = int(time.time() * 1000) - start_ms
            durations.append(elapsed)
            print(f"status={status} elapsed_ms={elapsed}")
            if status != "succeeded":
                print(json.dumps(task, ensure_ascii=False, indent=2), file=sys.stderr)
            break
        time.sleep(2)

durations.sort()
p50 = durations[(len(durations) - 1) // 2] if durations else 0
print()
print(f"Runs: {len(durations)}")
print(f"Durations (ms): {' '.join(str(d) for d in durations)}")
print(f"P50 (ms): {p50} (accept <= {accept_p50})")
if p50 <= accept_p50:
    print("PASS")
    sys.exit(0)
print("FAIL")
sys.exit(1)
PY

#!/usr/bin/env bash
# 30s load: 3x POST /v1/solve_async per second; sample worker container count. Author: kejiqing
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
if [[ -f "${ROOT}/.env" ]]; then
  set -a
  # shellcheck source=/dev/null
  source "${ROOT}/.env"
  set +a
fi
# shellcheck source=/dev/null
source "${ROOT}/deploy/podman/compose-include.sh"
URL="${1:-http://127.0.0.1:8088}"
DUR_SEC="${2:-30}"
REQ_LOG="$(mktemp)"
SAMPLE_LOG="$(mktemp)"
cleanup() { rm -f "$REQ_LOG" "$SAMPLE_LOG"; }
trap cleanup EXIT

worker_count() {
  local rt
  rt="$(claw_container_runtime_cli)" || return 0
  "${rt}" ps --format '{{.Names}}' 2>/dev/null | grep -c '^claw-gw-' || true
}

(
  tick=0
  while [[ $tick -lt "$DUR_SEC" ]]; do
    echo "$(date -Iseconds) tick=$((tick + 1)) claw_gw_running=$(worker_count)" >>"$SAMPLE_LOG"
    sleep 1
    tick=$((tick + 1))
  done
) &
SAMPLER_PID=$!

prompts=("ping" "ok" "hi" "pong" "yes")
for ((s = 1; s <= DUR_SEC; s++)); do
  for k in 1 2 3; do
    ds=$(( (s + k) % 3 + 1 ))
    p="${prompts[$((RANDOM % ${#prompts[@]}))]}"
    t0=$(python3 -c 'import time; print(time.time())')
    body=$(curl -sS -w '\n%{http_code}' -X POST "$URL/v1/solve_async" \
      -H 'Content-Type: application/json' \
      -d "{\"dsId\":${ds},\"userPrompt\":\"${p}\"}" 2>&1) || true
    t1=$(python3 -c 'import time; print(time.time())')
    code=$(echo "$body" | tail -n1)
    json=$(echo "$body" | sed '$d')
    tid=$(echo "$json" | python3 -c 'import sys,json; d=json.load(sys.stdin); print(d.get("taskId",""))' 2>/dev/null || echo "")
    printf '%s sec=%s n=%s ds=%s http=%s dt=%.4f tid=%s\n' "$(date -Iseconds)" "$s" "$k" "$ds" "$code" "$(python3 -c "print($t1-$t0)")" "$tid" >>"$REQ_LOG"
  done
  sleep 1
done
wait "$SAMPLER_PID" 2>/dev/null || true

echo "=== worker samples (running claw-gw-*) ==="
cat "$SAMPLE_LOG"
echo "=== accept-phase summary ==="
awk '{print $6}' "$REQ_LOG" | sort | uniq -c
python3 - <<PY
import re, statistics
from pathlib import Path
req = Path("$REQ_LOG").read_text().splitlines()
dts = []
codes = []
for line in req:
    m = re.search(r"http=(\d+)", line)
    if m: codes.append(int(m.group(1)))
    m = re.search(r"dt=([0-9.]+)", line)
    if m: dts.append(float(m.group(1)))
print("requests:", len(dts), "accept dt s: min", min(dts), "max", max(dts), "avg", statistics.mean(dts))
from collections import Counter
print("http codes:", dict(Counter(codes)))
PY

#!/usr/bin/env bash
# E2E: solve_async → poll hasReport (PG live chunks) → biz_advice_report SSE tail.
# Author: kejiqing
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
set -a
# shellcheck disable=SC1090
[[ -f "${REPO_ROOT}/.env" ]] && source "${REPO_ROOT}/.env"
set +a

PORT="${GATEWAY_HOST_PORT:-18088}"
BASE="http://127.0.0.1:${PORT}"
DS_ID="${DS_ID:-1}"
STORE_ID="${STORE_ID:-S20241007172800004204}"
QUESTION="${QUESTION:-最近生意怎样}"
POLL_SEC="${POLL_SEC:-3}"
MAX_POLLS="${MAX_POLLS:-120}"
REPORT_POLL_AFTER_HAS="${REPORT_POLL_AFTER_HAS:-1}"

echo "==> healthz ${BASE}"
curl -sf "${BASE}/healthz" | python3 -c 'import json,sys; o=json.load(sys.stdin); assert o.get("ok"); print("ok", o.get("solveIsolation", "?"))'

echo "==> init dsId=${DS_ID}"
curl -sf -X POST "${BASE}/v1/init" -H 'Content-Type: application/json' \
  -d "{\"dsId\":${DS_ID}}" >/dev/null

export DS_ID QUESTION STORE_ID
BODY="$(python3 -c '
import json, os
print(json.dumps({
    "dsId": int(os.environ["DS_ID"]),
    "userPrompt": os.environ["QUESTION"],
    "extraSession": {
        "store_id": os.environ["STORE_ID"],
        "tenant_code": "GPOS",
        "solution_code": "restaurant",
        "biz_type": "BOSS_REPORT",
    },
}, ensure_ascii=False))
')"

echo "==> solve_async"
ASYNC="$(curl -sf -X POST "${BASE}/v1/solve_async" -H 'Content-Type: application/json' -d "$BODY")"
echo "$ASYNC" | python3 -m json.tool
TASK_ID="$(echo "$ASYNC" | python3 -c 'import json,sys; print(json.load(sys.stdin)["taskId"])')"
SESSION_ID="$(echo "$ASYNC" | python3 -c 'import json,sys; print(json.load(sys.stdin)["sessionId"])')"
TURN_ID="$(echo "$ASYNC" | python3 -c 'import json,sys; print(json.load(sys.stdin)["turnId"])')"
echo "taskId=${TASK_ID} sessionId=${SESSION_ID} turnId=${TURN_ID}"

HAS_REPORT=false
REPORT_STARTED=false
for i in $(seq 1 "$MAX_POLLS"); do
  TASK="$(curl -sf "${BASE}/v1/tasks/${TASK_ID}")"
  read -r STATUS HAS_REPORT <<<"$(echo "$TASK" | python3 -c '
import json,sys
t=json.load(sys.stdin)
print(t.get("status",""), "true" if t.get("hasReport") else "false")
')"
  DESC="$(echo "$TASK" | python3 -c 'import json,sys; print(json.load(sys.stdin).get("currentTaskDesc") or "")')"
  echo "[poll $i] status=${STATUS} hasReport=${HAS_REPORT} desc=${DESC:0:60}"

  if [[ "$HAS_REPORT" == "true" && "$REPORT_STARTED" != "true" && "$REPORT_POLL_AFTER_HAS" == "1" ]]; then
    REPORT_STARTED=true
    echo "==> hasReport=true → sample report SSE (15s) …"
    timeout 15 curl -sN \
      "${BASE}/v1/biz_advice_report?sessionId=${SESSION_ID}&turnId=${TURN_ID}&dsId=${DS_ID}&stream=true" \
      | head -n 40 || true
    echo "… (SSE sample truncated)"
  fi

  case "$STATUS" in
    succeeded | failed | cancelled)
      echo "==> terminal status=${STATUS}"
      echo "$TASK" | python3 -m json.tool | tee "/tmp/claw-task-${TASK_ID}.json"
      if [[ "$STATUS" == "succeeded" ]]; then
        echo "==> report JSON (stream=false)"
        curl -sf \
          "${BASE}/v1/biz_advice_report?sessionId=${SESSION_ID}&turnId=${TURN_ID}&dsId=${DS_ID}&stream=false" \
          | python3 -m json.tool | head -n 30
      fi
      if [[ "$HAS_REPORT" != "true" && "$STATUS" == "succeeded" ]]; then
        echo "WARN: succeeded but hasReport was never true (check worker CLAW_GATEWAY_INTERNAL_* and TextDelta ingest)" >&2
        exit 1
      fi
      exit 0
      ;;
  esac
  sleep "$POLL_SEC"
done

echo "TIMEOUT task=${TASK_ID}" >&2
exit 1

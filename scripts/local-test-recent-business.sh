#!/usr/bin/env bash
# Local smoke (ds_id=1):「最近生意怎样」+ progressHistory. 结果记入 deploy/stack/claw-workspace/ds_1/TUNING-LOG.md
# Author: kejiqing
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# shellcheck disable=SC1090
source "${REPO_ROOT}/.env"

PORT="${GATEWAY_HOST_PORT:-18088}"
BASE="http://127.0.0.1:${PORT}"
DS_ID="${DS_ID:-1}"
STORE_ID="${STORE_ID:-S20241007172800004204}"
QUESTION="${QUESTION:-最近生意怎样}"
POLL_SEC="${POLL_SEC:-8}"
MAX_POLLS="${MAX_POLLS:-90}"

echo "==> gateway ${BASE}"
curl -sf "${BASE}/healthz" | python3 -c 'import json,sys; o=json.load(sys.stdin); assert o.get("ok"); print("healthz ok")'

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

echo "==> solve_async question=${QUESTION} store_id=${STORE_ID}"
ASYNC="$(curl -sf -X POST "${BASE}/v1/solve_async" -H 'Content-Type: application/json' -d "$BODY")"
echo "$ASYNC" | python3 -m json.tool
TASK_ID="$(echo "$ASYNC" | python3 -c 'import json,sys; print(json.load(sys.stdin)["taskId"])')"

echo "==> poll GET /v1/tasks/${TASK_ID} (every ${POLL_SEC}s, max ${MAX_POLLS})"
for i in $(seq 1 "$MAX_POLLS"); do
  TASK="$(curl -sf "${BASE}/v1/tasks/${TASK_ID}")"
  python3 -c '
import json, sys
t = json.load(sys.stdin)
st = t.get("status")
desc = t.get("currentTaskDesc") or ""
hist = t.get("progressHistory") or t.get("progress_history") or []
print(f"[{sys.argv[1]}] status={st} desc={desc!r} events={len(hist)}")
for e in hist[-5:]:
    k, m, ts = e.get("kind"), e.get("message"), e.get("tsMs")
    print(f"    - {k}: {m} @ {ts}")
' "$i" <<<"$TASK"
  STATUS="$(echo "$TASK" | python3 -c 'import json,sys; print(json.load(sys.stdin)["status"])')"
  case "$STATUS" in
    succeeded | failed | cancelled)
      echo "==> terminal status=${STATUS}"
      echo "$TASK" | python3 -m json.tool >"/tmp/claw-task-${TASK_ID}.json"
      echo "full task json: /tmp/claw-task-${TASK_ID}.json"
      if [[ "$STATUS" == "succeeded" ]]; then
        echo "$TASK" | python3 -c '
import json, sys
t = json.load(sys.stdin)
msg = (t.get("result") or {}).get("outputJson", {}).get("message") or ""
print("--- message preview (first 2000 chars) ---")
print(msg[:2000])
'
      fi
      exit 0
      ;;
  esac
  sleep "$POLL_SEC"
done

echo "TIMEOUT waiting for task ${TASK_ID}" >&2
exit 1

#!/usr/bin/env bash
# Purge stale gateway_turn_live_chunks (failed/cancelled age, succeeded leftovers, orphans).
# Author: kejiqing
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
set -a
# shellcheck disable=SC1090
[[ -f "${REPO_ROOT}/.env" ]] && source "${REPO_ROOT}/.env"
[[ -f "${REPO_ROOT}/deploy/stack/.env" ]] && source "${REPO_ROOT}/deploy/stack/.env"
set +a

DRY_RUN=0
FINISHED_AGE_HOURS=24
while [[ $# -gt 0 ]]; do
  case "$1" in
    --dry-run) DRY_RUN=1; shift ;;
    --finished-age-hours) FINISHED_AGE_HOURS="${2:?}"; shift 2 ;;
    *) echo "usage: $0 [--dry-run] [--finished-age-hours N]" >&2; exit 2 ;;
  esac
done

URL="${CLAW_GATEWAY_DATABASE_URL:-}"
if [[ -z "$URL" ]]; then
  echo "CLAW_GATEWAY_DATABASE_URL is required" >&2
  exit 1
fi

NOW_MS="$(python3 -c 'import time; print(int(time.time()*1000))')"
CUTOFF_MS="$(python3 -c "h=int('${FINISHED_AGE_HOURS}'); print(int('${NOW_MS}') - h*3600*1000)")"

echo "purge live_chunks: finished_age_hours=${FINISHED_AGE_HOURS} cutoff_ms=${CUTOFF_MS} dry_run=${DRY_RUN}"

run_sql() {
  local label="$1"
  local sql="$2"
  if [[ "$DRY_RUN" == "1" ]]; then
    echo "-- dry-run ${label}"
    echo "$sql"
    psql "$URL" -c "SELECT count(*) AS would_delete FROM (${sql//;/}) q;" 2>/dev/null || true
  else
    echo "==> ${label}"
    psql "$URL" -v ON_ERROR_STOP=1 -c "$sql"
  fi
}

run_sql "failed/cancelled terminal" \
  "DELETE FROM gateway_turn_live_chunks c USING gateway_turns t
   WHERE c.turn_id = t.turn_id AND t.status IN ('failed','cancelled')
   AND t.finished_at_ms IS NOT NULL AND t.finished_at_ms < ${CUTOFF_MS};"

run_sql "succeeded leftover chunks" \
  "DELETE FROM gateway_turn_live_chunks c USING gateway_turns t
   WHERE c.turn_id = t.turn_id AND t.status = 'succeeded';"

run_sql "orphan chunks" \
  "DELETE FROM gateway_turn_live_chunks WHERE turn_id NOT IN (SELECT turn_id FROM gateway_turns);"

echo "done"

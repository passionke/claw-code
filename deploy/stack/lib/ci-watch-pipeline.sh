#!/usr/bin/env bash
# Poll Sunmi GitLab until the newest pipeline for REF finishes; print job summary.
# Usage: ./deploy/stack/lib/ci-watch-pipeline.sh [ref]   # default: current branch
# Requires: glab auth login --hostname code.sunmi.com
# Author: kejiqing
set -euo pipefail

REF="${1:-$(git rev-parse --abbrev-ref HEAD 2>/dev/null || echo main)}"
PROJECT="minidata/claw-code"
POLL_SEC="${CLAW_CI_WATCH_POLL_SEC:-30}"
TIMEOUT_SEC="${CLAW_CI_WATCH_TIMEOUT_SEC:-7200}"

if ! command -v glab >/dev/null 2>&1; then
  echo "error: install glab (brew install glab) and: glab auth login --hostname code.sunmi.com" >&2
  exit 1
fi

latest_pipeline() {
  glab api "projects/minidata%2Fclaw-code/pipelines?ref=${REF}&per_page=1" 2>/dev/null \
    | python3 -c 'import json,sys; d=json.load(sys.stdin); print(d[0]["id"], d[0]["status"], d[0]["sha"][:8]) if d else sys.exit(1)'
}

echo "==> watching ${PROJECT} ref=${REF} (poll=${POLL_SEC}s timeout=${TIMEOUT_SEC}s)"
started=$(date +%s)
pid=""
pstatus=""
psha=""
while true; do
  read -r pid pstatus psha < <(latest_pipeline) || {
    echo "no pipeline for ref=${REF} yet; sleep ${POLL_SEC}s…"
    sleep "${POLL_SEC}"
    continue
  }
  now=$(date +%s)
  if ((now - started > TIMEOUT_SEC)); then
    echo "error: timeout waiting pipeline #${pid}" >&2
    exit 1
  fi
  echo "--- pipeline #${pid} ${pstatus} sha=${psha} ($(date -Is 2>/dev/null || date))"
  glab api "projects/minidata%2Fclaw-code/pipelines/${pid}/jobs" \
    | python3 -c 'import json,sys
for j in sorted(json.load(sys.stdin), key=lambda x: x.get("stage","")):
    print(f"  {j[\"status\"]:12} {j[\"stage\"]:12} {j[\"name\"]:22} id={j[\"id\"]}")'
  case "${pstatus}" in
    success | failed | canceled | skipped) break ;;
    *) sleep "${POLL_SEC}" ;;
  esac
done

printf '==> final: pipeline #%s %s sha=%s\n' "${pid}" "${pstatus}" "${psha}"
if [[ "${pstatus}" != success ]]; then
  failed_id="$(glab api "projects/minidata%2Fclaw-code/pipelines/${pid}/jobs" \
    | python3 -c 'import json,sys
for j in json.load(sys.stdin):
    if j["status"]=="failed":
        print(j["id"]); break')"
  if [[ -n "${failed_id}" ]]; then
    echo "==> failed job trace tail (id=${failed_id}):"
    glab api "projects/minidata%2Fclaw-code/jobs/${failed_id}/trace" | tail -40
  fi
  exit 1
fi

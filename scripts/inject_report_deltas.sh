#!/usr/bin/env bash
# Inject report.delta into gateway hub for a running turn (dev verify). Author: kejiqing
set -euo pipefail
TURN_ID="${1:?turn id}"
GATEWAY="${GATEWAY:-http://127.0.0.1:18088}"
TOKEN="${CLAW_GATEWAY_INTERNAL_TOKEN:-claw-internal-dev-token}"
TEXT="${2:-这是一段用于验证真流式显示的门店 S20241007172800004204 经营摘要。}"
CHUNK="${CHUNK:-24}"
i=0
while [ "$i" -lt "${#TEXT}" ]; do
  part="${TEXT:i:CHUNK}"
  i=$((i + CHUNK))
  body=$(printf '{"ev":"report.delta","text":%s}' "$(python3 -c 'import json,sys; print(json.dumps(sys.argv[1]))' "$part")")
  curl -sS -o /dev/null -w "." -X POST \
    -H "Authorization: Bearer ${TOKEN}" \
    -H "Content-Type: application/json" \
    -d "$body" \
    "${GATEWAY}/v1/internal/turns/${TURN_ID}/stdout-event"
  sleep 0.08
done
echo
curl -sS -o /dev/null -X POST \
  -H "Authorization: Bearer ${TOKEN}" \
  -H "Content-Type: application/json" \
  -d '{"ev":"solve.done","clawExitCode":0,"outputText":"ok","outputJson":{"message":"done"}}' \
  "${GATEWAY}/v1/internal/turns/${TURN_ID}/stdout-event"
echo "done inject ${TURN_ID}"

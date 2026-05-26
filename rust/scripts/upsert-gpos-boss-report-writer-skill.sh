#!/usr/bin/env bash
# Upsert GPOS_BOSS_REPORT_WRITER skill on a claw gateway (default ds_id=1). Author: kejiqing
#
# Default skill body: http-gateway-rs crate skills/ (same file as gateway include_str! fallback).
# Optional locale for demos only: CLAW_BOSS_REPORT_SKILL_LOCALE=ja → gpos-boss-report-writer.ja.SKILL.md
set -euo pipefail

GATEWAY_BASE="${CLAW_GATEWAY_BASE:-http://192.168.9.252:18088}"
DS_ID="${CLAW_DS_ID:-1}"
SKILL_NAME="GPOS_BOSS_REPORT_WRITER"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
GATEWAY_CRATE_DIR="$(cd "${SCRIPT_DIR}/../crates/http-gateway-rs" && pwd)"
SKILL_MD="${GATEWAY_CRATE_DIR}/skills/gpos-boss-report-writer.SKILL.md"
if [[ "${CLAW_BOSS_REPORT_SKILL_LOCALE:-zh}" == "ja" ]]; then
  SKILL_MD="${SCRIPT_DIR}/gpos-boss-report-writer.ja.SKILL.md"
fi

if [[ ! -f "$SKILL_MD" ]]; then
  echo "missing skill template: $SKILL_MD" >&2
  exit 1
fi

skill_content="$(cat "$SKILL_MD")"
payload="$(python3 -c 'import json,sys; print(json.dumps({"skillName":sys.argv[1],"skillContent":sys.argv[2]}))' \
  "$SKILL_NAME" "$skill_content")"

curl_args=(-sS -f -H "Content-Type: application/json" -X POST "${GATEWAY_BASE}/v1/project/skills/${DS_ID}" -d "$payload")
if [[ -n "${CLAW_GATEWAY_BEARER_TOKEN:-}" ]]; then
  curl_args+=(-H "Authorization: Bearer ${CLAW_GATEWAY_BEARER_TOKEN}")
fi

curl "${curl_args[@]}"

echo ""
echo "OK: ${SKILL_NAME} on ds_${DS_ID} at ${GATEWAY_BASE} (source ${SKILL_MD})"

#!/usr/bin/env bash
# Pre-release GPOS router split helper — ⚠️ 仅运维发布窗口使用；日常验收用 apply_local.sh
# Requires: GW, CLAW_ADMIN_TOKEN, ROUTER_PROJ, KB_PROJ; OPS_PROJ defaults to 271.
# 本任务约束：不要对预发 252/271 执行，除非维护者明确授权。
set -euo pipefail

: "${GW:?set GW e.g. http://192.168.9.252:18088}"
: "${CLAW_ADMIN_TOKEN:?set CLAW_ADMIN_TOKEN}"
: "${ROUTER_PROJ:?set ROUTER_PROJ (new router project id)}"
: "${KB_PROJ:?set KB_PROJ (new kb-qa project id)}"
OPS_PROJ="${OPS_PROJ:-271}"
AUTH="Authorization: Bearer ${CLAW_ADMIN_TOKEN}"
JSON='Content-Type: application/json'

echo "==> Set router role (seeds CLAUDE + specialist-registry)"
curl -fsS -X PUT "${GW}/v1/projects/${ROUTER_PROJ}/role" \
  -H "${AUTH}" -H "${JSON}" \
  -d '{"projectRole":"router"}'

echo "==> Router delegate-targets (kb + ops)"
curl -fsS -X PUT "${GW}/v1/projects/${ROUTER_PROJ}/delegate-targets" \
  -H "${AUTH}" -H "${JSON}" \
  -d "{\"targets\":[{\"targetProjId\":${KB_PROJ},\"label\":\"kb-qa\",\"capabilityHint\":\"product manual\"},{\"targetProjId\":${OPS_PROJ},\"label\":\"ops-analysis\",\"capabilityHint\":\"business analytics\"}]}"

echo "==> Activate router stable (materialize registry appendix)"
curl -fsS -X POST "${GW}/v1/project/config/${ROUTER_PROJ}/versions/activate" \
  -H "${AUTH}" -H "${JSON}" \
  -d '{}' || true

echo "Done. Point BFF / eval at ROUTER_PROJ=${ROUTER_PROJ} (export GPOS_PROJ_ID=${ROUTER_PROJ})"

#!/usr/bin/env bash
# Kill all sandboxes on self-hosted / FC e2b (orphan cleanup after gateway restarts). Author: kejiqing
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
# shellcheck source=/dev/null
[[ -f "${ROOT_DIR}/.env" ]] && source "${ROOT_DIR}/.env"

API_URL="${CLAW_FC_API_URL:-http://10.8.0.9:3000}"
API_KEY="${CLAW_FC_API_KEY:-${ALIYUN_E2B_TOKEN:-}}"

fail() { echo "fc-sandbox-cleanup: $*" >&2; exit 1; }

[[ -n "${API_KEY}" ]] || fail "CLAW_FC_API_KEY or ALIYUN_E2B_TOKEN required"

list_json="$(curl -sS -m 15 "${API_URL%/}/sandboxes" -H "X-API-Key: ${API_KEY}")" \
  || fail "GET ${API_URL}/sandboxes failed"

ids=()
while IFS= read -r line; do
  [[ -n "${line}" ]] && ids+=("${line}")
done < <(python3 - <<PY
import json, sys
raw = """${list_json}"""
try:
    data = json.loads(raw)
except json.JSONDecodeError as e:
    print(f"parse error: {e}", file=sys.stderr)
    sys.exit(1)
if not isinstance(data, list):
    print("unexpected sandboxes payload (expected JSON array)", file=sys.stderr)
    sys.exit(1)
for row in data:
    sid = row.get("sandboxID") or row.get("sandboxId") or ""
    if sid:
        print(sid)
PY
)

echo "fc-sandbox-cleanup: ${#ids[@]} sandbox(es) on ${API_URL}"
if ((${#ids[@]} == 0)); then
  echo "fc-sandbox-cleanup: OK (nothing to kill)"
  exit 0
fi

killed=0
for sid in "${ids[@]}"; do
  code="$(curl -sS -o /dev/null -w '%{http_code}' -m 15 -X DELETE \
    "${API_URL%/}/sandboxes/${sid}" -H "X-API-Key: ${API_KEY}" || true)"
  if [[ "${code}" == "204" || "${code}" == "200" || "${code}" == "404" ]]; then
    echo "  killed ${sid} (HTTP ${code})"
    killed=$((killed + 1))
  else
    echo "  warn: ${sid} DELETE HTTP ${code}" >&2
  fi
done

echo "fc-sandbox-cleanup: OK (killed ${killed}/${#ids[@]})"

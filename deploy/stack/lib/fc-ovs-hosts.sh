#!/usr/bin/env bash
# Print /etc/hosts line for e2b OVS browser (self-hosted IP domain). Author: kejiqing
set -euo pipefail

LIB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${LIB_DIR}/../../.." && pwd)"
# shellcheck disable=SC1090
[[ -f "${ROOT_DIR}/.env" ]] && source "${ROOT_DIR}/.env"

GATEWAY_PORT="${GATEWAY_HOST_PORT:-8088}"
PROJ_ID="${1:-1}"

tmp="$(mktemp)"
trap 'rm -f "${tmp}"' EXIT
curl -fsS "http://127.0.0.1:${GATEWAY_PORT}/v1/projects/${PROJ_ID}/ovs/workspace" >"${tmp}"
python3 - "${tmp}" <<'PY'
import json, sys
with open(sys.argv[1], encoding="utf-8") as f:
    d = json.load(f)
print("ovsFolderUrl:", d.get("ovsFolderUrl", ""))
print("ovsBrowserHostsLine:", d.get("ovsBrowserHostsLine", ""))
print()
print("# Add to /etc/hosts (sudo), then open ovsFolderUrl in browser:")
print(d.get("ovsBrowserHostsLine", "(missing — CLAW_OVS_BACKEND=e2b?)"))
PY

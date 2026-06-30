#!/usr/bin/env bash
# e2b OVS: runtime install claw-vscode + traffic verify. Author: kejiqing
set -euo pipefail

LIB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJ_ID="${CLAW_E2B_E2E_PROJ_ID:-3}"

"${LIB_DIR}/install-claw-vscode-e2b-ovs.sh" --proj-id "${PROJ_ID}" "$@"

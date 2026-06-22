#!/usr/bin/env bash
# FC OVS: runtime install claw-vscode + traffic verify. Author: kejiqing
set -euo pipefail

LIB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJ_ID="${CLAW_FC_E2E_PROJ_ID:-3}"

"${LIB_DIR}/install-claw-vscode-fc-ovs.sh" --proj-id "${PROJ_ID}" "$@"

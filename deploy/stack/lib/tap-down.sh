#!/usr/bin/env bash
# Stop claude-tap only (gateway unchanged). Author: kejiqing
set -euo pipefail

LIB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PODMAN_DIR="$(cd "${LIB_DIR}/.." && pwd)"

# shellcheck source=/dev/null
source "${LIB_DIR}/claude-tap-local.sh"
claw_claude_tap_stop "${PODMAN_DIR}"

echo "claude-tap stopped (gateway stack untouched)."

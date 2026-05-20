#!/usr/bin/env bash
# Legacy: stop gateway + tap. Prefer: gateway.sh down && gateway.sh tap-down. Author: kejiqing
set -euo pipefail
LIB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
"${LIB_DIR}/down.sh"
"${LIB_DIR}/tap-down.sh"

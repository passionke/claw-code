#!/usr/bin/env bash
# Legacy: tap + gateway. Prefer: gateway.sh tap-up && gateway.sh up. Author: kejiqing
set -euo pipefail
LIB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
"${LIB_DIR}/tap-up.sh"
"${LIB_DIR}/up.sh" "$@"

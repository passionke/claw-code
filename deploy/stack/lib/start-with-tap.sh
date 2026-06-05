#!/usr/bin/env bash
# Legacy: tap + gateway. Prefer: gateway.sh up (local profile auto tap-up). Author: kejiqing
set -euo pipefail
LIB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
"${LIB_DIR}/tap-up.sh"
"${LIB_DIR}/up.sh" "$@"

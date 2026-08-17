#!/usr/bin/env bash
# Bootstrap kb-qa project (99011) for local router acceptance. Author: kejiqing
set -euo pipefail
exec python3 "$(dirname "$0")/apply_kb_local.py" "$@"

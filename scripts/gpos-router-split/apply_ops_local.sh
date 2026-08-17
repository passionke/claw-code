#!/usr/bin/env bash
# Bootstrap ops-analysis project (99012) for local router acceptance. Author: kejiqing
set -euo pipefail
exec python3 "$(dirname "$0")/apply_ops_local.py" "$@"

#!/usr/bin/env bash
# CI cluster gate — Python implementation (HTTP solve_async, no shell capture). Author: kejiqing
set -euo pipefail
exec python3 "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/ci_cluster_solve_e2e.py" "$@"

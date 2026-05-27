#!/usr/bin/env bash
# Deprecated: use ./deploy/stack/gateway.sh up. Author: kejiqing
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
exec bash "${ROOT}/deploy/stack/lib/up.sh" "$@"

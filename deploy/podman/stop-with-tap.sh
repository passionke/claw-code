#!/usr/bin/env bash
# Deprecated: use ./deploy/stack/gateway.sh down && ./deploy/stack/gateway.sh tap-down. Author: kejiqing
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
exec bash "${ROOT}/deploy/stack/lib/stop-with-tap.sh" "$@"

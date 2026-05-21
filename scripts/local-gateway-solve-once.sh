#!/usr/bin/env bash
# Deprecated: use ./deploy/stack/gateway.sh solve-once-local
# Author: kejiqing
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
exec "${ROOT}/deploy/stack/gateway.sh" solve-once-local "$@"

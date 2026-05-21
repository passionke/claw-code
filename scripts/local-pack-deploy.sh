#!/usr/bin/env bash
# Deprecated: use ./deploy/stack/gateway.sh pack-deploy
# Author: kejiqing
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
exec "${ROOT}/deploy/stack/gateway.sh" pack-deploy "$@"

#!/usr/bin/env sh
# Deprecated: canonical copy is deploy/stack/lib/claw-gateway-worker.sh
# Author: kejiqing
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
exec "${SCRIPT_DIR}/../deploy/stack/lib/claw-gateway-worker.sh"

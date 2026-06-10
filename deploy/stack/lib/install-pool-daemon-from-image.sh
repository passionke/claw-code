#!/usr/bin/env bash
# Deprecated: gateway image no longer ships pool daemon (use linux-compile → .linux-artifacts/claw-sandbox).
# Author: kejiqing
set -euo pipefail

echo "error: pool daemon is no longer extracted from gateway image (pool_outside / claw-sandbox)." >&2
echo "hint: run ./deploy/stack/gateway.sh build (linux-compile writes deploy/stack/.linux-artifacts/release/claw-sandbox)" >&2
echo "      or set CLAW_POOL_DAEMON_BIN to a host-built claw-sandbox binary" >&2
exit 1

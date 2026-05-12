#!/usr/bin/env bash
# Smoke: `./deploy/stack/gateway.sh up` (or `lib/up.sh`) starts host daemon on TCP + gateway with CLAW_POOL_DAEMON_TCP.
# Author: kejiqing
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../../.." && pwd)"
cd "$ROOT/rust"
cargo build -q -p http-gateway-rs --bin claw-pool-daemon --bin http-gateway-rs
echo "Prefer: cd $ROOT && ./deploy/stack/gateway.sh up"
echo "Daemon (manual): CLAW_POOL_DAEMON_TCP_BIND=0.0.0.0:9943 + CLAW_WORK_ROOT=... + claw-pool-daemon"
echo "Gateway env: CLAW_POOL_DAEMON_TCP=host.containers.internal:9943 + CLAW_POOL_RPC_HOST_WORK_ROOT=<host workspace path>"
echo "curl -sS http://127.0.0.1:8088/healthz | jq '.poolRpcRemote,.poolRpcTcp'"

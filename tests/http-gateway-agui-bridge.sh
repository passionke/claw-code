#!/usr/bin/env bash
# M2: gateway event tap + ag-ui modules. Author: kejiqing
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "${ROOT}/rust"
cargo test -p http-gateway-rs --lib -- agui auth_audit 2>&1
echo "http-gateway-agui-bridge: ok"

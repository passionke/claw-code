#!/usr/bin/env bash
# Deprecated wrapper — use tests/http-gateway-pool-consumer-chain.sh
# Author: kejiqing
set -euo pipefail
exec "$(cd "$(dirname "$0")" && pwd)/http-gateway-pool-consumer-chain.sh" "$@"

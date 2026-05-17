#!/usr/bin/env bash
# M0 self-check: contract skeleton exists. Author: kejiqing
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
for f in README.md L0-shared-identifiers.md L1-opencode-to-agui.md \
  L2-agui-to-gateway.md L3-gateway-to-worker.md L4-interrupts.md L5-auth-audit.md; do
  test -f "${ROOT}/docs/contracts/${f}"
done
echo "contracts-m0: ok"

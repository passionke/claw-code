#!/usr/bin/env bash
# Deprecated: use install-claw-sandbox-from-release.sh via gateway.sh up --release. Author: kejiqing
set -euo pipefail

echo "error: pool daemon is no longer extracted from gateway image (pool_outside / claw-sandbox)." >&2
echo "hint: ./deploy/stack/gateway.sh up --release release-vX.Y.Z  (pulls claw-sandbox image + installs host binary)" >&2
exit 1

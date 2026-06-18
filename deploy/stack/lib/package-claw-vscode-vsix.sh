#!/usr/bin/env bash
# Package extensions/claw-vscode as VSIX. Author: kejiqing
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
exec "${ROOT_DIR}/deploy/stack/lib/package-ovs-extension-vsix.sh" \
  "${ROOT_DIR}/extensions/claw-vscode" \
  "${ROOT_DIR}/deploy/stack/claw.claw-vscode-0.2.0.vsix"

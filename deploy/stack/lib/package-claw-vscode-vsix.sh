#!/usr/bin/env bash
# Package extensions/claw-vscode as VSIX. Author: kejiqing
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
EXT_VER="$(python3 -c "import json; print(json.load(open('${ROOT_DIR}/extensions/claw-vscode/package.json'))['version'])")"
exec "${ROOT_DIR}/deploy/stack/lib/package-ovs-extension-vsix.sh" \
  "${ROOT_DIR}/extensions/claw-vscode" \
  "${ROOT_DIR}/deploy/stack/claw.claw-vscode-${EXT_VER}.vsix"

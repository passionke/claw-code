#!/usr/bin/env bash
# Static release-boundary gate. Author: kejiqing
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

reject() {
  local pattern="$1"
  shift
  if grep -En "$pattern" "$@"; then
    echo "release boundary violation: /${pattern}/" >&2
    exit 1
  fi
}

reject 'claw-worker|publish-worker-protocol|agent-engines' \
  "$ROOT/.github/workflows/claw-code-image.yaml" \
  "$ROOT/deploy/stack/lib/ci-publish-nora.sh"

reject 'claw-code|gateway-playground|ci-publish-nora|agent-engines/' \
  "$ROOT/.github/workflows/e2b-worker-protocol.yml" \
  "$ROOT/deploy/e2b/publish-worker-protocol.sh"

reject 'gateway|publish-worker-protocol|claw-worker-base|deploy/pack' \
  "$ROOT/deploy/agent-engines/upload-raw.sh" \
  "$ROOT/deploy/agent-engines/opencode/build.sh" \
  "$ROOT/deploy/agent-engines/codex-acp/build.sh"

if [[ -e "$ROOT/deploy/pack/publish.sh" || -e "$ROOT/.github/workflows/pack-publish.yml" ]]; then
  echo "release boundary violation: mixed publish entry still exists" >&2
  exit 1
fi

echo "release boundaries: ok"

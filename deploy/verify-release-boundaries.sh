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
  "$ROOT/deploy/stack/lib/ci-publish-nora.sh" \
  "$ROOT/deploy/jenkins/claw-code-nora.Jenkinsfile"

reject 'gateway-playground|agent-engines/|stack/lib/ci-publish-nora' \
  "$ROOT/.github/workflows/e2b-worker-protocol.yml" \
  "$ROOT/deploy/e2b/publish-worker-protocol.sh" \
  "$ROOT/deploy/e2b/ci-publish-nora.sh" \
  "$ROOT/deploy/jenkins/claw-e2b-protocol-nora.Jenkinsfile"

reject 'publish-worker-protocol|debian-bookworm-claw-worker|claw-worker-base|stack/lib/ci-publish-nora|deploy/pack' \
  "$ROOT/deploy/agent-engines/upload-raw.sh" \
  "$ROOT/deploy/agent-engines/ci-publish-nora.sh" \
  "$ROOT/deploy/agent-engines/opencode/build.sh" \
  "$ROOT/deploy/agent-engines/codex-acp/build.sh" \
  "$ROOT/deploy/jenkins/claw-agent-engine-nora.Jenkinsfile"

if [[ -e "$ROOT/deploy/pack/publish.sh" || -e "$ROOT/.github/workflows/pack-publish.yml" ]]; then
  echo "release boundary violation: mixed publish entry still exists" >&2
  exit 1
fi

echo "release boundaries: ok"

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
  "$ROOT/deploy/stack/lib/ci-publish-gateway-nora.sh" \
  "$ROOT/deploy/stack/lib/ci-publish-playground-nora.sh" \
  "$ROOT/deploy/jenkins/claw-code-nora.Jenkinsfile" \
  "$ROOT/deploy/jenkins/claw-code-gateway-nora.Jenkinsfile" \
  "$ROOT/deploy/jenkins/claw-code-playground-nora.Jenkinsfile"

reject 'gateway-playground|agent-engines/|stack/lib/ci-publish-nora|ci-publish-gateway-nora|ci-publish-playground-nora' \
  "$ROOT/.github/workflows/e2b-worker-protocol.yml" \
  "$ROOT/deploy/e2b/publish-worker-protocol.sh" \
  "$ROOT/deploy/e2b/ci-publish-nora.sh" \
  "$ROOT/deploy/jenkins/claw-e2b-protocol-nora.Jenkinsfile"

reject 'publish-worker-protocol|claw-worker-base|debian-bookworm-claw-worker|stack/lib/ci-publish-nora|ci-publish-gateway-nora|ci-publish-playground-nora|deploy/pack' \
  "$ROOT/deploy/agent-engines/upload-raw.sh" \
  "$ROOT/deploy/agent-engines/ci-publish-nora.sh" \
  "$ROOT/deploy/agent-engines/opencode/build.sh" \
  "$ROOT/deploy/agent-engines/appserver/build.sh" \
  "$ROOT/deploy/jenkins/claw-agent-engine-nora.Jenkinsfile"

# neuro-harness-acp is an agent-engine; must not appear in Gateway publish paths.
if [[ -f "$ROOT/deploy/agent-engines/neuro-harness-acp/build.sh" ]]; then
  reject 'publish-worker-protocol|claw-worker-base|debian-bookworm-claw-worker|stack/lib/ci-publish-nora|ci-publish-gateway-nora|ci-publish-playground-nora|deploy/pack' \
    "$ROOT/deploy/agent-engines/neuro-harness-acp/build.sh"
fi

if [[ -e "$ROOT/deploy/pack/publish.sh" || -e "$ROOT/.github/workflows/pack-publish.yml" ]]; then
  echo "release boundary violation: mixed publish entry still exists" >&2
  exit 1
fi

echo "release boundaries: ok"

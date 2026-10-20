#!/usr/bin/env bash
# Path gate for split Gateway/Admin Jobs. Author: kejiqing
# Usage: COMPONENT=gateway|playground FORCE=0|1 bash ci-publish-path-gate.sh
# Exit 0 = publish; exit 10 = skip (no matching paths). Formal tags always publish.
set -euo pipefail

COMPONENT="${COMPONENT:?COMPONENT=gateway|playground required}"
FORCE="${FORCE:-0}"
ROOT="$(cd "$(dirname "$0")/../../.." && pwd)"

if [[ "$FORCE" == "1" ]]; then
  echo "path-gate: FORCE=1 → publish ${COMPONENT}"
  exit 0
fi

RELEASE_TAG="${RELEASE_TAG:-${GIT_TAG:-}}"
if [[ "$RELEASE_TAG" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ || "$RELEASE_TAG" =~ ^release-v[0-9] ]]; then
  echo "path-gate: formal tag ${RELEASE_TAG} → publish ${COMPONENT}"
  exit 0
fi

# Changed paths in HEAD (works with shallow single-commit checkout). Author: kejiqing
mapfile -t CHANGED < <(git -C "$ROOT" show --name-only --pretty=format: HEAD | sed '/^$/d' || true)
if [[ ${#CHANGED[@]} -eq 0 ]]; then
  echo "path-gate: no changed files listed; publish ${COMPONENT} (safe default)"
  exit 0
fi

matches_gateway() {
  local f="$1"
  case "$f" in
    rust/*|deploy/stack/Containerfile.gateway-rs.prebuilt|deploy/stack/lib/linux-compile.sh|deploy/stack/lib/rust-compile-image.sh|deploy/stack/lib/ci-publish-gateway-nora.sh|deploy/stack/lib/ci-publish-common-nora.sh|deploy/stack/lib/container-build.sh|deploy/stack/debian-bookworm-ustc.sources|deploy/stack/rust-version.env|deploy/e2b/e2b_exec.py|deploy/e2b/guest-install-agent-engine.sh|deploy/e2b/ensure-worker-resource-template.py|deploy/e2b/e2b_template_build.py|deploy/e2b/e2b_template_registry.py|deploy/e2b/requirements-e2b-sdk.txt|scripts/mind-kb/*|docs/gpos-assistant-prompt-content.md|deploy/jenkins/claw-code-gateway-nora.Jenkinsfile)
      return 0
      ;;
  esac
  return 1
}

matches_playground() {
  local f="$1"
  case "$f" in
    web/gateway-admin/*|web/gateway-async-playground/*|deploy/stack/Containerfile.gateway-playground|deploy/stack/lib/ci-publish-playground-nora.sh|deploy/stack/lib/ci-publish-common-nora.sh|deploy/jenkins/claw-code-playground-nora.Jenkinsfile)
      return 0
      ;;
  esac
  return 1
}

hit=0
for f in "${CHANGED[@]}"; do
  case "$COMPONENT" in
    gateway)
      if matches_gateway "$f"; then hit=1; break; fi
      ;;
    playground)
      if matches_playground "$f"; then hit=1; break; fi
      ;;
    *)
      echo "path-gate: unknown COMPONENT=${COMPONENT}" >&2
      exit 2
      ;;
  esac
done

if [[ "$hit" -eq 1 ]]; then
  echo "path-gate: ${COMPONENT} paths changed → publish"
  exit 0
fi

echo "path-gate: no ${COMPONENT} paths in HEAD → skip"
printf '  %s\n' "${CHANGED[@]}" | head -n 40
exit 10

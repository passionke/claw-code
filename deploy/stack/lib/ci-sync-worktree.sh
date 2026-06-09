#!/usr/bin/env bash
# GitLab runner: sync worktree to pipeline commit and derive CLAW_RELEASE_TAG.
# Source from .gitlab-ci.yml: source ./deploy/stack/lib/ci-sync-worktree.sh
# Author: kejiqing
set -euo pipefail

LIB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${LIB_DIR}/../../.." && pwd)"
cd "${CI_PROJECT_DIR:-${REPO_ROOT}}"

git fetch origin "${CI_COMMIT_REF_NAME}"
git checkout -f "${CI_COMMIT_SHA}"

if [[ "${CI_COMMIT_BRANCH:-}" == "main" ]]; then
  export CLAW_RELEASE_TAG="release-${CI_COMMIT_SHORT_SHA}"
else
  export CLAW_RELEASE_TAG="release-${CI_COMMIT_REF_SLUG}-${CI_COMMIT_SHORT_SHA}"
fi

echo "==> ci worktree ${CI_COMMIT_REF_NAME}@${CI_COMMIT_SHORT_SHA} CLAW_RELEASE_TAG=${CLAW_RELEASE_TAG}"

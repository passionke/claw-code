#!/usr/bin/env bash
# GitHub Actions self-hosted runner: sync worktree to pipeline commit and derive CLAW_RELEASE_TAG.
# Source from workflow: source ./deploy/stack/lib/ci-sync-worktree-github.sh
# Author: kejiqing
set -euo pipefail

LIB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${LIB_DIR}/../../.." && pwd)"
cd "${GITHUB_WORKSPACE:-${REPO_ROOT}}"

ref_input="${INPUT_REF:-${GITHUB_REF_NAME:-main}}"
sha="${GITHUB_SHA:?GITHUB_SHA required}"

git fetch origin "${ref_input}" --depth=1 2>/dev/null || git fetch origin "${ref_input}"
if git rev-parse --verify "origin/${ref_input}" >/dev/null 2>&1; then
  git checkout -f "origin/${ref_input}"
  sha="$(git rev-parse HEAD)"
elif git rev-parse --verify "${ref_input}" >/dev/null 2>&1; then
  git checkout -f "${ref_input}"
  sha="$(git rev-parse HEAD)"
else
  git checkout -f "${sha}"
fi

ref_name="${ref_input}"
ref_slug="${ref_name//\//-}"
short_sha="${sha:0:8}"

if [[ "${ref_name}" == "main" ]]; then
  export CLAW_RELEASE_TAG="release-${short_sha}"
else
  export CLAW_RELEASE_TAG="release-${ref_slug}-${short_sha}"
fi

echo "==> github ci worktree ${ref_name}@${short_sha} CLAW_RELEASE_TAG=${CLAW_RELEASE_TAG}"

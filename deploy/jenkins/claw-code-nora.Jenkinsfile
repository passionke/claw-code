// Author: kejiqing
// Gateway/Admin only → Nora. Source of truth for job claw-code-nora.
// Formal: GIT_TAG=vX.Y.Z. Fast (main): GIT_REF=<branch|sha> → publish sha-<12>.
pipeline {
  agent { label 'home29' }
  options {
    disableConcurrentBuilds()
  }
  parameters {
    string(name: 'GIT_TAG', defaultValue: '', description: 'Formal gitea tag vX.Y.Z (never release-v*). Empty when using GIT_REF.')
    string(name: 'GIT_REF', defaultValue: '', description: 'Fast path: branch or commit SHA on gitea (publishes sha-<12>). Empty when using GIT_TAG.')
  }
  environment {
    NEXUS_PUSH_REGISTRY = 'nora.home.passionke.top'
    NEXUS_NS = 'passionke'
    REGION = 'china'
    DOCKER_BUILDKIT = '0'
    CLAW_LINUX_COMPILE_PLATFORM = 'linux/amd64'
    TARGETARCH = 'amd64'
    CLAW_REPO = 'https://code.passionke.top/passionke/claw-code.git'
  }
  stages {
    stage('Checkout') {
      steps {
        sh '''#!/bin/bash
set -euo pipefail
echo "==> wipe workspace (root-owned compile debris)"
docker run --rm -v "${WORKSPACE}:/w" alpine:3.20 \
  sh -c 'rm -rf /w/* /w/.[!.]* /w/..?* 2>/dev/null || true'
git clone "${CLAW_REPO}" .
# Prefer gitea remote name when present; CLAW_REPO already points at gitea. Author: kejiqing
git fetch --tags --force origin
git fetch --force origin '+refs/heads/*:refs/remotes/origin/*' || true

TAG_TRIM="$(printf '%s' "${GIT_TAG:-}" | tr -d '[:space:]')"
REF_TRIM="$(printf '%s' "${GIT_REF:-}" | tr -d '[:space:]')"

if [[ -n "$TAG_TRIM" ]]; then
  echo "==> formal checkout tag ${TAG_TRIM}"
  git checkout -f "tags/${TAG_TRIM}" 2>/dev/null || git checkout -f "${TAG_TRIM}"
  git describe --tags --exact-match HEAD
  export RELEASE_TAG="${TAG_TRIM}"
elif [[ -n "$REF_TRIM" ]]; then
  echo "==> fast checkout ref ${REF_TRIM}"
  git checkout -f "${REF_TRIM}"
  SHA12="$(git rev-parse --short=12 HEAD)"
  export RELEASE_TAG="sha-${SHA12}"
else
  echo "error: set GIT_TAG=vX.Y.Z or GIT_REF=<branch|sha>" >&2
  exit 2
fi

echo "==> HEAD=$(git rev-parse HEAD) RELEASE_TAG=${RELEASE_TAG}"
git log -1 --oneline
test -f rust/Cargo.toml
chmod +x deploy/stack/lib/ci-publish-nora.sh deploy/verify-release-boundaries.sh
# Persist for next stage (same agent workspace). Author: kejiqing
printf '%s\n' "${RELEASE_TAG}" > .claw-release-tag
'''
      }
    }
    stage('Publish Gateway/Admin to Nora') {
      steps {
        withCredentials([usernamePassword(credentialsId: 'nora-deployer', usernameVariable: 'NEXUS_USER', passwordVariable: 'NEXUS_PASSWORD')]) {
          sh '''#!/bin/bash
set -euo pipefail
RELEASE_TAG="$(cat .claw-release-tag)"
export RELEASE_TAG
export GIT_TAG="${RELEASE_TAG}"
bash deploy/stack/lib/ci-publish-nora.sh
'''
        }
      }
    }
  }
}

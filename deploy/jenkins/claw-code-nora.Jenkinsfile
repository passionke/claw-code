// Author: kejiqing
// MANUAL all-in-one (Gateway + Playground). Prefer split Jobs started by
// Gitea Actions: claw-code-gateway-nora / claw-code-playground-nora.
// No GenericTrigger.
pipeline {
  agent { label 'nora-pack' }
  options {
    disableConcurrentBuilds()
  }
  parameters {
    string(name: 'GIT_TAG', defaultValue: '', description: 'Formal gitea tag vX.Y.Z (never release-v*). Empty when using GIT_REF.')
    string(name: 'GIT_REF', defaultValue: '', description: 'Fast path: branch or commit SHA on gitea.')
    booleanParam(
      name: 'CARGO_CACHE_RESET',
      defaultValue: false,
      description: 'Wipe durable cargo-registry/git/sccache before compile.'
    )
  }
  environment {
    NEXUS_PUSH_REGISTRY = 'nora.home.passionke.top'
    NEXUS_NS = 'passionke'
    ACR_REGISTRY = 'crpi-cf9vxpq3n8or17mw.cn-hangzhou.personal.cr.aliyuncs.com/passionke'
    REGION = 'china'
    DOCKER_BUILDKIT = '0'
    CLAW_LINUX_COMPILE_PLATFORM = 'linux/amd64'
    TARGETARCH = 'amd64'
    CLAW_REPO = 'https://code.passionke.top/passionke/claw-code.git'
    CARGO_INCREMENTAL = '0'
    CLAW_DISABLE_SCCACHE = '1'
  }
  stages {
    stage('Checkout') {
      steps {
        sh '''#!/bin/bash
set -euo pipefail
echo "==> wipe workspace (root-owned compile debris)"
docker run --rm -v "${WORKSPACE}:/w" alpine:3.20 \\
  sh -c 'rm -rf /w/* /w/.[!.]* /w/..?* 2>/dev/null || true'

TAG_TRIM="$(printf '%s' "${GIT_TAG:-}" | tr -d '[:space:]')"
REF_TRIM="$(printf '%s' "${GIT_REF:-}" | tr -d '[:space:]')"

if [[ -n "$TAG_TRIM" ]]; then
  echo "==> shallow clone tag ${TAG_TRIM}"
  git clone --depth 1 --branch "${TAG_TRIM}" "${CLAW_REPO}" .
  git describe --tags --exact-match HEAD
  export RELEASE_TAG="${TAG_TRIM}"
elif [[ -n "$REF_TRIM" ]]; then
  echo "==> shallow fetch ref ${REF_TRIM}"
  git init
  git remote add origin "${CLAW_REPO}"
  git fetch --depth 1 origin "${REF_TRIM}"
  git checkout -f FETCH_HEAD
  SHA12="$(git rev-parse --short=12 HEAD)"
  export RELEASE_TAG="sha-${SHA12}"
else
  echo "error: set GIT_TAG=vX.Y.Z or GIT_REF=<branch|sha>" >&2
  exit 2
fi

echo "==> HEAD=$(git rev-parse HEAD) RELEASE_TAG=${RELEASE_TAG}"
git log -1 --oneline
test -f rust/Cargo.toml
chmod +x deploy/stack/lib/ci-publish-nora.sh \\
  deploy/stack/lib/ci-publish-gateway-nora.sh \\
  deploy/stack/lib/ci-publish-playground-nora.sh \\
  deploy/verify-release-boundaries.sh
printf '%s\\n' "${RELEASE_TAG}" > .claw-release-tag
'''
      }
    }
    stage('Publish Gateway+Admin to Nora+ACR') {
      steps {
        withCredentials([
          usernamePassword(credentialsId: 'nora-deployer', usernameVariable: 'NEXUS_USER', passwordVariable: 'NEXUS_PASSWORD'),
          usernamePassword(credentialsId: 'acr-deployer', usernameVariable: 'ACR_USERNAME', passwordVariable: 'ACR_PASSWORD')
        ]) {
          sh '''#!/bin/bash
set -euo pipefail
RELEASE_TAG="$(cat .claw-release-tag)"
export RELEASE_TAG
export GIT_TAG="${RELEASE_TAG}"
if [[ "${CARGO_CACHE_RESET:-false}" == "true" ]]; then
  export CLAW_CARGO_CACHE_RESET=1
fi
bash deploy/stack/lib/ci-publish-nora.sh
'''
        }
      }
    }
  }
}

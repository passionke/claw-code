// Author: kejiqing
// Gateway/Admin only → Nora. Source of truth for job claw-code-nora.
pipeline {
  agent { label 'home29' }
  options {
    disableConcurrentBuilds()
  }
  parameters {
    string(name: 'GIT_TAG', defaultValue: 'release-v2.0.35', description: 'Tag to publish: vX.Y.Z or release-v*')
  }
  environment {
    NEXUS_PUSH_REGISTRY = 'nora.home.passionke.top'
    NEXUS_NS = 'passionke'
    REGION = 'china'
    DOCKER_BUILDKIT = '0'
    CLAW_LINUX_COMPILE_PLATFORM = 'linux/amd64'
    TARGETARCH = 'amd64'
    // GitHub is source of truth; gitea mirror is 8h and not on this path. Author: kejiqing
    CLAW_REPO = 'https://github.com/passionke/claw-code.git'
  }
  stages {
    stage('Checkout tag') {
      steps {
        sh '''#!/bin/bash
set -euo pipefail
echo "==> wipe workspace (root-owned compile debris)"
docker run --rm -v "${WORKSPACE}:/w" alpine:3.20 \
  sh -c 'rm -rf /w/* /w/.[!.]* /w/..?* 2>/dev/null || true'
echo "==> clone ${CLAW_REPO} @ ${GIT_TAG}"
git clone "${CLAW_REPO}" .
git fetch --tags --force origin
git checkout -f "tags/${GIT_TAG}" 2>/dev/null || git checkout -f "${GIT_TAG}"
git describe --tags --exact-match HEAD
git log -1 --oneline
test -f rust/Cargo.toml
chmod +x deploy/stack/lib/ci-publish-nora.sh deploy/verify-release-boundaries.sh
'''
      }
    }
    stage('Publish Gateway/Admin to Nora') {
      steps {
        withCredentials([usernamePassword(credentialsId: 'nora-deployer', usernameVariable: 'NEXUS_USER', passwordVariable: 'NEXUS_PASSWORD')]) {
          sh '''#!/bin/bash
set -euo pipefail
export RELEASE_TAG="${GIT_TAG}"
export GIT_TAG="${GIT_TAG}"
bash deploy/stack/lib/ci-publish-nora.sh
'''
        }
      }
    }
  }
}

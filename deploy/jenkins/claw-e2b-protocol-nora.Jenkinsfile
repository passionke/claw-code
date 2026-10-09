// Author: kejiqing
// e2b Worker protocol only → Nora + e2b template register. Manual job.
pipeline {
  agent { label 'home29' }
  options {
    disableConcurrentBuilds()
  }
  parameters {
    string(name: 'GIT_TAG', defaultValue: 'release-v2.0.30', description: 'Tag that contains the protocol sources')
  }
  environment {
    REGION = 'china'
    DOCKER_BUILDKIT = '0'
    CLAW_LINUX_COMPILE_PLATFORM = 'linux/amd64'
    TARGETARCH = 'amd64'
    CLAW_IMAGE_PREFIX = 'nora.home.passionke.top/passionke'
    CLAW_E2B_API_URL = 'http://e2b.home.passionke.top:3000'
    CLAW_E2B_SANDBOX_URL = 'http://e2b.home.passionke.top:3002'
    CLAW_REPO = 'https://code.passionke.top/passionke/claw-code.git'
  }
  stages {
    stage('Checkout tag') {
      steps {
        sh '''#!/bin/bash
set -euo pipefail
docker run --rm -v "${WORKSPACE}:/w" alpine:3.20 \
  sh -c 'rm -rf /w/* /w/.[!.]* /w/..?* 2>/dev/null || true'
git clone "${CLAW_REPO}" .
git fetch --tags --force origin
git checkout -f "tags/${GIT_TAG}" 2>/dev/null || git checkout -f "${GIT_TAG}"
git describe --tags --exact-match HEAD
git log -1 --oneline
chmod +x deploy/e2b/ci-publish-nora.sh deploy/e2b/publish-worker-protocol.sh deploy/verify-release-boundaries.sh
'''
      }
    }
    stage('Publish e2b protocol to Nora') {
      steps {
        withCredentials([usernamePassword(credentialsId: 'nora-deployer', usernameVariable: 'NEXUS_USER', passwordVariable: 'NEXUS_PASSWORD')]) {
          sh '''#!/bin/bash
set -euo pipefail
export RELEASE_TAG="${GIT_TAG}"
export GIT_TAG="${GIT_TAG}"
bash deploy/e2b/ci-publish-nora.sh
'''
        }
      }
    }
  }
}

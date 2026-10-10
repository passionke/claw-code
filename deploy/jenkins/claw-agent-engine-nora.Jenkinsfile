// Author: kejiqing
// One Agent engine → Nora raw. Manual job. Never touches Gateway or e2b protocol.
pipeline {
  agent { label 'home29' }
  options {
    disableConcurrentBuilds()
  }
  parameters {
    string(name: 'GIT_TAG', defaultValue: 'release-v2.0.35', description: 'Tag that contains engine build scripts')
    string(name: 'ENGINE_ID', defaultValue: 'opencode', description: 'Directory under deploy/agent-engines/')
    string(name: 'ENGINE_VERSION', defaultValue: '1.18.34', description: 'Version label without arch, e.g. 1.18.34')
    string(name: 'OPENCODE_VERSION', defaultValue: '1.18.34', description: 'Passed to opencode/build.sh when ENGINE_ID=opencode')
    string(name: 'CODEX_ACP_VERSION', defaultValue: '2.1.1', description: 'Passed to codex-acp/build.sh when ENGINE_ID=codex-acp')
  }
  environment {
    REGION = 'china'
    TARGETARCH = 'amd64'
    RAW_BASE = 'https://nora.home.passionke.top/raw/claw-agent-engines'
    // GitHub is source of truth; gitea mirror is 8h and not on this path. Author: kejiqing
    CLAW_REPO = 'https://github.com/passionke/claw-code.git'
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
chmod +x deploy/agent-engines/ci-publish-nora.sh deploy/agent-engines/upload-raw.sh \
  deploy/agent-engines/*/build.sh deploy/verify-release-boundaries.sh
'''
      }
    }
    stage('Build + upload Agent engine') {
      steps {
        withCredentials([usernamePassword(credentialsId: 'nora-deployer', usernameVariable: 'RAW_USERNAME', passwordVariable: 'RAW_PASSWORD')]) {
          sh '''#!/bin/bash
set -euo pipefail
export ENGINE_VERSION="${ENGINE_VERSION}"
export OPENCODE_VERSION="${OPENCODE_VERSION}"
export CODEX_ACP_VERSION="${CODEX_ACP_VERSION}"
export RAW_BASE="${RAW_BASE}"
export TARGETARCH="${TARGETARCH}"
bash deploy/agent-engines/ci-publish-nora.sh "${ENGINE_ID}"
'''
        }
      }
    }
  }
}

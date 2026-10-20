// Author: kejiqing
// One Agent engine → Nora raw. Manual job. Never touches Gateway or e2b protocol.
pipeline {
  // Shared nora-pack: home29 preferred when online; built-in failover. Author: kejiqing
  agent { label 'nora-pack' }
  options {
    disableConcurrentBuilds()
  }
  parameters {
    string(name: 'GIT_TAG', defaultValue: 'v2.0.28', description: 'gitea tag vX.Y.Z only (never release-v*)')
    string(name: 'ENGINE_ID', defaultValue: 'opencode', description: 'Directory under deploy/agent-engines/ (opencode|appserver|neuro-harness-acp)')
    string(name: 'ENGINE_VERSION', defaultValue: '1.18.34', description: 'Version label without arch, e.g. 1.18.34')
    string(name: 'OPENCODE_VERSION', defaultValue: '1.18.34', description: 'Passed to opencode/build.sh when ENGINE_ID=opencode')
    string(name: 'APPSERVER_VERSION', defaultValue: '2.1.1', description: 'Passed to appserver/build.sh when ENGINE_ID=appserver')
    string(name: 'NEURO_HARNESS_ACP_VERSION', defaultValue: '0.1.0', description: 'Passed to neuro-harness-acp/build.sh when ENGINE_ID=neuro-harness-acp')
  }
  environment {
    REGION = 'china'
    TARGETARCH = 'amd64'
    RAW_BASE = 'https://nora.home.passionke.top/raw/claw-agent-engines'
    CLAW_REPO = 'https://code.passionke.top/passionke/claw-code.git'
  }
  stages {
    stage('Checkout tag') {
      steps {
        sh '''#!/bin/bash
set -euo pipefail
docker run --rm -v "${WORKSPACE}:/w" alpine:3.20 \
  sh -c 'rm -rf /w/* /w/.[!.]* /w/..?* 2>/dev/null || true'
# Shallow tag only — avoid full-history clone. Author: kejiqing
git clone --depth 1 --branch "${GIT_TAG}" "${CLAW_REPO}" .
git describe --tags --exact-match HEAD
git log -1 --oneline
chmod +x deploy/agent-engines/ci-publish-nora.sh deploy/agent-engines/upload-raw.sh \
  deploy/agent-engines/opencode/build.sh deploy/agent-engines/appserver/build.sh \
  deploy/agent-engines/neuro-harness-acp/build.sh deploy/verify-release-boundaries.sh
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
export APPSERVER_VERSION="${APPSERVER_VERSION}"
export NEURO_HARNESS_ACP_VERSION="${NEURO_HARNESS_ACP_VERSION}"
export RAW_BASE="${RAW_BASE}"
export TARGETARCH="${TARGETARCH}"
export CLAW_DISABLE_SCCACHE=1
bash deploy/agent-engines/ci-publish-nora.sh "${ENGINE_ID}"
'''
        }
      }
    }
  }
}

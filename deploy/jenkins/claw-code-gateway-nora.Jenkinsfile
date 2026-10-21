// Author: kejiqing
// Gateway only → Nora + ACR. Path-gated on GIT_REF; always on v*.
// Started by Gitea Actions (home-neurogate-*), not a repo webhook.
// Formal: GIT_TAG=vX.Y.Z. Main: GIT_REF=<sha> → sha-<12> + :latest.
pipeline {
  agent { label 'nora-pack' }
  options {
    disableConcurrentBuilds()
  }
  parameters {
    string(name: 'GIT_TAG', defaultValue: '', description: 'Formal gitea tag vX.Y.Z (never release-v*). Empty when using GIT_REF.')
    string(name: 'GIT_REF', defaultValue: '', description: 'Fast path: branch or commit SHA on gitea (publishes sha-<12> + latest). Empty when using GIT_TAG.')
    booleanParam(
      name: 'CARGO_CACHE_RESET',
      defaultValue: false,
      description: 'Wipe durable cargo-registry/git/sccache before compile.'
    )
    booleanParam(
      name: 'FORCE_PUBLISH',
      defaultValue: false,
      description: 'Ignore path gate (always publish).'
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

if [[ -z "$TAG_TRIM" && -z "$REF_TRIM" ]]; then
  WREF="$(printf '%s' "${ref:-}" | tr -d '[:space:]')"
  WAFTER="$(printf '%s' "${after:-}" | tr -d '[:space:]')"
  WSHA="$(printf '%s' "${hook_sha:-}" | tr -d '[:space:]')"
  if [[ "$WREF" =~ ^refs/tags/(v[0-9]+[.][0-9]+[.][0-9]+)$ ]]; then
    TAG_TRIM="${BASH_REMATCH[1]}"
  elif [[ "$WREF" =~ ^v[0-9]+[.][0-9]+[.][0-9]+$ ]]; then
    TAG_TRIM="$WREF"
  elif [[ "$WREF" =~ ^refs/heads/.+ ]]; then
    if [[ -n "$WAFTER" && "$WAFTER" != "0000000000000000000000000000000000000000" ]]; then
      REF_TRIM="$WAFTER"
    elif [[ -n "$WSHA" ]]; then
      REF_TRIM="$WSHA"
    fi
  fi
fi

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
  echo "error: set GIT_TAG=vX.Y.Z or GIT_REF=<branch|sha> (or Gitea webhook ref)" >&2
  exit 2
fi

echo "==> HEAD=$(git rev-parse HEAD) RELEASE_TAG=${RELEASE_TAG}"
git log -1 --oneline
test -f rust/Cargo.toml
chmod +x deploy/stack/lib/ci-publish-gateway-nora.sh \\
  deploy/stack/lib/ci-publish-common-nora.sh \\
  deploy/stack/lib/ci-publish-path-gate.sh \\
  deploy/verify-release-boundaries.sh
printf '%s\\n' "${RELEASE_TAG}" > .claw-release-tag
'''
      }
    }
    stage('Path gate') {
      steps {
        sh '''#!/bin/bash
set -euo pipefail
RELEASE_TAG="$(cat .claw-release-tag)"
export RELEASE_TAG
FORCE=0
if [[ "${FORCE_PUBLISH:-false}" == "true" ]]; then FORCE=1; fi
set +e
COMPONENT=gateway FORCE="$FORCE" bash deploy/stack/lib/ci-publish-path-gate.sh
RC=$?
set -e
if [[ "$RC" -eq 10 ]]; then
  echo "==> skip gateway publish (path gate)"
  printf 'skip\\n' > .claw-publish-decision
  exit 0
fi
if [[ "$RC" -ne 0 ]]; then
  exit "$RC"
fi
printf 'publish\\n' > .claw-publish-decision
'''
      }
    }
    stage('Publish Gateway to Nora+ACR') {
      when {
        expression { return readFile('.claw-publish-decision').trim() == 'publish' }
      }
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
  echo "==> CLAW_CARGO_CACHE_RESET=1"
fi
bash deploy/stack/lib/ci-publish-gateway-nora.sh
'''
        }
      }
    }
  }
}

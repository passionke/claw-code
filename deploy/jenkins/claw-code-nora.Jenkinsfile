// Author: kejiqing
// Gateway/Admin → Nora + ACR (same tags). Source of truth for job claw-code-nora.
// Formal: GIT_TAG=vX.Y.Z. Fast: any branch push → GIT_REF=<sha> → publish sha-<12>.
// Pack does not require merge to main. neurogate up stays main-only (Actions).
// Gitea webhook → GenericTrigger (same token as repo hook). Manual builds use params.
pipeline {
  agent { label 'home29' }
  options {
    disableConcurrentBuilds()
  }
  parameters {
    string(name: 'GIT_TAG', defaultValue: '', description: 'Formal gitea tag vX.Y.Z (never release-v*). Empty when using GIT_REF.')
    string(name: 'GIT_REF', defaultValue: '', description: 'Fast path: branch or commit SHA on gitea (publishes sha-<12>). Empty when using GIT_TAG.')
    booleanParam(
      name: 'SCCACHE_RESET',
      defaultValue: false,
      description: 'Wipe durable sccache (~/claw-ci-cache/claw-code/sccache) before compile. Compile also auto-resets once on failure.'
    )
  }
  triggers {
    // Token must match Gitea hook …/generic-webhook-trigger/invoke?token=… Author: kejiqing
    GenericTrigger(
      genericVariables: [
        [key: 'ref', value: '$.ref'],
        [key: 'after', value: '$.after'],
        [key: 'hook_sha', value: '$.sha']
      ],
      causeString: 'Gitea webhook $ref',
      token: 'cab0cf265cdb70d5bc725a9cd69ac7c1ebadaaed',
      printContributedVariables: true,
      printPostContent: false,
      regexpFilterText: '$ref',
      regexpFilterExpression: 'refs/heads/.+|refs/tags/v[0-9]+[.][0-9]+[.][0-9]+|^v[0-9]+[.][0-9]+[.][0-9]+$'
    )
  }
  environment {
    NEXUS_PUSH_REGISTRY = 'nora.home.passionke.top'
    NEXUS_NS = 'passionke'
    // Dual-write same tags to personal ACR (home-ubt / gateway default prefix). Author: kejiqing
    ACR_REGISTRY = 'crpi-cf9vxpq3n8or17mw.cn-hangzhou.personal.cr.aliyuncs.com/passionke'
    REGION = 'china'
    DOCKER_BUILDKIT = '0'
    CLAW_LINUX_COMPILE_PLATFORM = 'linux/amd64'
    TARGETARCH = 'amd64'
    CLAW_REPO = 'https://code.passionke.top/passionke/claw-code.git'
    // Stabilize home29 release compiles (linker/rustc SIGSEGV under parallel pressure). Author: kejiqing
    CARGO_BUILD_JOBS = '8'
    RUST_MIN_STACK = '268435456'
    CARGO_PROFILE_RELEASE_CODEGEN_UNITS = '8'
    // home29: rustc/LLVM SIGSEGV under opt-level=3 (hyper/h2 GlobalsAA). Author: kejiqing
    CARGO_PROFILE_RELEASE_OPT_LEVEL = '2'
    CARGO_INCREMENTAL = '0'
    CLAW_DISABLE_SCCACHE = '1'
    CLAW_LINUX_LINKER = 'lld'
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

# Gitea GenericTrigger contributes ref / after / hook_sha when params empty. Author: kejiqing
if [[ -z "$TAG_TRIM" && -z "$REF_TRIM" ]]; then
  WREF="$(printf '%s' "${ref:-}" | tr -d '[:space:]')"
  WAFTER="$(printf '%s' "${after:-}" | tr -d '[:space:]')"
  WSHA="$(printf '%s' "${hook_sha:-}" | tr -d '[:space:]')"
  if [[ "$WREF" =~ ^refs/tags/(v[0-9]+[.][0-9]+[.][0-9]+)$ ]]; then
    TAG_TRIM="${BASH_REMATCH[1]}"
  elif [[ "$WREF" =~ ^v[0-9]+[.][0-9]+[.][0-9]+$ ]]; then
    TAG_TRIM="$WREF"
  elif [[ "$WREF" =~ ^refs/heads/.+ ]]; then
    # Any branch push → pack that commit (no need to merge main first). Author: kejiqing
    if [[ -n "$WAFTER" && "$WAFTER" != "0000000000000000000000000000000000000000" ]]; then
      REF_TRIM="$WAFTER"
    elif [[ -n "$WSHA" ]]; then
      REF_TRIM="$WSHA"
    fi
  fi
fi

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
  echo "error: set GIT_TAG=vX.Y.Z or GIT_REF=<branch|sha> (or Gitea webhook ref)" >&2
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
    stage('Publish Gateway/Admin to Nora+ACR') {
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
# booleanParam becomes "true"/"false" strings. Author: kejiqing
if [[ "${SCCACHE_RESET:-false}" == "true" ]]; then
  export CLAW_SCCACHE_RESET=1
  echo "==> CLAW_SCCACHE_RESET=1 (Jenkins SCCACHE_RESET)"
fi
echo "==> CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS:-} RUST_MIN_STACK=${RUST_MIN_STACK:-}"
bash deploy/stack/lib/ci-publish-nora.sh
'''
        }
      }
    }
  }
}

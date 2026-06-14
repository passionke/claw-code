#!/usr/bin/env bash
# CI runner disk maintenance (GitLab / GitHub self-hosted): Rust target, Docker cache, old release-* tags.
# Does NOT remove claw-postgres-data or claw-workspace (running stack / PG SoT).
# Author: kejiqing
set -euo pipefail

LIB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${LIB_DIR}/../../.." && pwd)"
cd "${CI_PROJECT_DIR:-${GITHUB_WORKSPACE:-${REPO_ROOT}}}"

KEEP_RELEASE_TAGS="${CLAW_CI_KEEP_RELEASE_TAGS:-15}"
BUILDER_PRUNE_UNTIL_HOURS="${CLAW_CI_BUILDER_PRUNE_UNTIL_HOURS:-168}"

log_section() {
  echo ""
  echo "==> $*"
}

df_line() {
  df -h / 2>/dev/null | tail -1 || true
}

docker_df() {
  if command -v docker >/dev/null 2>&1; then
    docker system df 2>/dev/null || true
  fi
}

log_section "disk-prune start ($(date -Is 2>/dev/null || date))"
echo "    root fs: $(df_line)"
docker_df

log_section "rust/build artifacts (debug target + .linux-artifacts)"
if [[ -x "${REPO_ROOT}/deploy/stack/gateway.sh" ]]; then
  "${REPO_ROOT}/deploy/stack/gateway.sh" clean --debug-only
else
  echo "    gateway.sh missing; skip"
fi

log_section "docker dangling images"
if command -v docker >/dev/null 2>&1; then
  docker image prune -f 2>/dev/null || true
else
  echo "    docker not in PATH; skip"
fi

log_section "docker build cache (older than ${BUILDER_PRUNE_UNTIL_HOURS}h)"
if command -v docker >/dev/null 2>&1; then
  docker builder prune -f --filter "until=${BUILDER_PRUNE_UNTIL_HOURS}h" 2>/dev/null \
    || docker builder prune -f 2>/dev/null \
    || echo "    builder prune not supported or failed (non-fatal)"
else
  echo "    docker not in PATH; skip"
fi

prune_old_release_tags() {
  local prefix="${CLAW_IMAGE_PREFIX:-local}"
  local repo keep n tag
  for repo in claw-code claw-gateway-worker claw-gateway-worker-relaxed claw-gateway-playground; do
    mapfile -t tags < <(
      docker images "${prefix}/${repo}" --format '{{.Tag}}' 2>/dev/null \
        | grep -E '^release-' \
        | sort -r
    ) || true
    n="${#tags[@]}"
    if [[ "${n}" -le "${KEEP_RELEASE_TAGS}" ]]; then
      echo "    ${prefix}/${repo}: ${n} release tag(s), keep all (limit ${KEEP_RELEASE_TAGS})"
      continue
    fi
    echo "    ${prefix}/${repo}: ${n} release tags, removing $((n - KEEP_RELEASE_TAGS)) oldest"
    for ((i = KEEP_RELEASE_TAGS; i < n; i++)); do
      tag="${tags[$i]}"
      docker rmi -f "${prefix}/${repo}:${tag}" 2>/dev/null \
        && echo "      removed ${prefix}/${repo}:${tag}" \
        || echo "      skip ${prefix}/${repo}:${tag} (in use?)" >&2
    done
  done
}

log_section "old release-* image tags (keep newest ${KEEP_RELEASE_TAGS} per repo)"
if command -v docker >/dev/null 2>&1; then
  prune_old_release_tags
else
  echo "    docker not in PATH; skip"
fi

log_section "disk-prune done"
echo "    root fs: $(df_line)"
docker_df
echo "    untouched: deploy/stack/claw-postgres-data deploy/stack/claw-workspace (and running :local images)"

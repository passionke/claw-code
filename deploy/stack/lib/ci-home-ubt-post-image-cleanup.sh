#!/usr/bin/env bash
# Post image-build on home-ubt: free disk without wiping compile cache.
# Author: kejiqing
#
# Keeps: $HOME/claw-ci-cache/claw-code (cargo registry + sccache + swagger zip)
# Drops: linux-artifacts, claw-ci-artifacts for this run, dangling docker images,
#        buildx cache, local package tags already pushed to ACR
set -euo pipefail

if [[ "${RUNNER_ENVIRONMENT:-}" != "self-hosted" ]]; then
  echo "ci-home-ubt-post-image-cleanup: skip (not self-hosted)"
  exit 0
fi

WS="${GITHUB_WORKSPACE:-}"
RUN_ID="${GITHUB_RUN_ID:-}"

echo "==> home-ubt post-image cleanup (keep \$HOME/claw-ci-cache)"

if [[ -n "${WS}" ]]; then
  for path in \
    "${WS}/deploy/stack/.linux-artifacts" \
    "${WS}/rust/target"; do
    if [[ -d "${path}" ]]; then
      echo "remove ${path}"
      rm -rf "${path}" 2>/dev/null \
        || docker run --rm -v "${WS}:/w:rw" alpine:3.20 \
          rm -rf "/w/${path#"${WS}"/}" \
        || true
    fi
  done
fi

if [[ -n "${RUN_ID}" && -d "${HOME}/claw-ci-artifacts/${RUN_ID}" ]]; then
  echo "remove ${HOME}/claw-ci-artifacts/${RUN_ID}"
  rm -rf "${HOME}/claw-ci-artifacts/${RUN_ID}" 2>/dev/null || true
fi

# Stale stash dirs from cancelled runs (best-effort, keep newest 2)
if [[ -d "${HOME}/claw-ci-artifacts" ]]; then
  # shellcheck disable=SC2012
  mapfile -t old < <(ls -1dt "${HOME}/claw-ci-artifacts"/* 2>/dev/null | tail -n +3 || true)
  for d in "${old[@]:-}"; do
    [[ -n "${d}" ]] || continue
    echo "remove stale stash ${d}"
    rm -rf "${d}" 2>/dev/null || true
  done
fi

if command -v docker >/dev/null 2>&1; then
  # Local tags we just pushed to ACR — safe to drop (ACR is SoT)
  for repo in claw-code claw-gateway-worker claw-gateway-worker-relaxed claw-gateway-playground claw-rust-compile; do
    mapfile -t imgs < <(docker images "${repo}" --format '{{.Repository}}:{{.Tag}}' 2>/dev/null || true)
    for img in "${imgs[@]:-}"; do
      [[ -n "${img}" && "${img}" != *"<none>"* ]] || continue
      echo "rmi ${img}"
      docker rmi -f "${img}" 2>/dev/null || true
    done
  done

  echo "docker image prune"
  docker image prune -f 2>/dev/null || true
  echo "docker builder prune (until=24h)"
  docker builder prune -f --filter "until=24h" 2>/dev/null \
    || docker builder prune -f 2>/dev/null \
    || true
  docker system df 2>/dev/null || true
fi

df -h / 2>/dev/null | tail -1 || true
echo "==> home-ubt post-image cleanup done"

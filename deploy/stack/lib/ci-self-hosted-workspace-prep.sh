#!/usr/bin/env bash
# Pre-checkout on self-hosted runners: drop root-owned linux-compile debris so
# actions/checkout can reset the workspace (EACCES on .linux-artifacts/release/build).
# No-op on github-hosted (ephemeral). Author: kejiqing
set -euo pipefail

if [[ "${RUNNER_ENVIRONMENT:-}" != "self-hosted" ]]; then
  exit 0
fi

WS="${GITHUB_WORKSPACE:-}"
if [[ -z "${WS}" || ! -d "${WS}" ]]; then
  exit 0
fi

echo "ci self-hosted prep: workspace=${WS} user=$(id -un) uid=$(id -u)"

# linux-compile runs cargo as root in docker; stale build/ dirs block checkout clean.
ART="${WS}/deploy/stack/.linux-artifacts"
if [[ -d "${ART}" ]]; then
  echo "ci self-hosted prep: remove ${ART}"
  rm -rf "${ART}" 2>/dev/null || sudo rm -rf "${ART}"
fi

# Best-effort: fix ownership on any leftover root-owned paths under workspace.
if ! touch "${WS}/.ci-self-hosted-prep" 2>/dev/null; then
  echo "ci self-hosted prep: chown workspace (checkout clean blocked)"
  sudo chown -R "$(id -u):$(id -g)" "${WS}"
fi
rm -f "${WS}/.ci-self-hosted-prep" 2>/dev/null || true

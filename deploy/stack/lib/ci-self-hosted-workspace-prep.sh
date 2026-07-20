#!/usr/bin/env bash
# Pre-checkout on self-hosted: remove root-owned linux-compile debris (no passwordless sudo).
# Uses docker when plain rm fails. No-op on github-hosted. Author: kejiqing
set -euo pipefail

if [[ "${RUNNER_ENVIRONMENT:-}" != "self-hosted" ]]; then
  exit 0
fi

WS="${GITHUB_WORKSPACE:-}"
if [[ -z "${WS}" || ! -d "${WS}" ]]; then
  exit 0
fi

RUN_UID="$(id -u)"
RUN_GID="$(id -g)"

docker_root() {
  docker run --rm -v "${WS}:/w:rw" alpine:3.20 "$@"
}

ART="${WS}/deploy/stack/.linux-artifacts"
if [[ -d "${ART}" ]]; then
  echo "ci self-hosted prep: remove ${ART}"
  rm -rf "${ART}" 2>/dev/null || docker_root rm -rf /w/deploy/stack/.linux-artifacts
fi

if ! touch "${WS}/.ci-self-hosted-prep" 2>/dev/null; then
  echo "ci self-hosted prep: chown ${WS} via docker"
  docker_root chown -R "${RUN_UID}:${RUN_GID}" /w
fi
rm -f "${WS}/.ci-self-hosted-prep" 2>/dev/null || true

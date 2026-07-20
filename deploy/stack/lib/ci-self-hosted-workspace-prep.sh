#!/usr/bin/env bash
# Pre-checkout on self-hosted: docker chown full workspace + drop linux-artifacts. Author: kejiqing
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

echo "ci self-hosted prep: chown ${WS}"
docker_root chown -R "${RUN_UID}:${RUN_GID}" /w

ART="${WS}/deploy/stack/.linux-artifacts"
if [[ -d "${ART}" ]]; then
  echo "ci self-hosted prep: remove ${ART}"
  rm -rf "${ART}" 2>/dev/null || docker_root rm -rf /w/deploy/stack/.linux-artifacts
fi

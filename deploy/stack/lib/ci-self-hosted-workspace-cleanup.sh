#!/usr/bin/env bash
# Post-job on self-hosted: drop linux-compile output (docker fallback, no sudo). Author: kejiqing
set -euo pipefail

if [[ "${RUNNER_ENVIRONMENT:-}" != "self-hosted" ]]; then
  exit 0
fi

WS="${GITHUB_WORKSPACE:-}"
ART="${WS}/deploy/stack/.linux-artifacts"
if [[ -n "${WS}" && -d "${ART}" ]]; then
  echo "ci self-hosted cleanup: remove ${ART}"
  rm -rf "${ART}" 2>/dev/null || docker run --rm -v "${WS}:/w:rw" alpine:3.20 rm -rf /w/deploy/stack/.linux-artifacts || true
fi

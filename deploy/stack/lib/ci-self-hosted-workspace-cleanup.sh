#!/usr/bin/env bash
# Post-job on self-hosted: remove linux-compile output so the next job checkout stays clean.
# Author: kejiqing
set -euo pipefail

if [[ "${RUNNER_ENVIRONMENT:-}" != "self-hosted" ]]; then
  exit 0
fi

WS="${GITHUB_WORKSPACE:-}"
if [[ -z "${WS}" || ! -d "${WS}" ]]; then
  exit 0
fi

ART="${WS}/deploy/stack/.linux-artifacts"
if [[ -d "${ART}" ]]; then
  echo "ci self-hosted cleanup: remove ${ART}"
  rm -rf "${ART}" 2>/dev/null || sudo rm -rf "${ART}" || true
fi

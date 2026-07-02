#!/usr/bin/env bash
# Ensure e2b observe-singleton on e2b (template startCmd). Author: kejiqing
set -euo pipefail

LIB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${LIB_DIR}/../../.." && pwd)"
ENV_FILE="${REPO_ROOT}/.env"

if [[ ! -f "${ENV_FILE}" ]]; then
  echo "missing ${ENV_FILE} — copy from deploy/stack/env.selfhosted-e2b.example" >&2
  exit 1
fi

# shellcheck disable=SC1090
set -a && source "${ENV_FILE}" && set +a

exec python3 "${REPO_ROOT}/deploy/e2b/e2b-tap-live-up.py" "$@"

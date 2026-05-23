#!/usr/bin/env bash
# Start claude-tap only (gateway: ./deploy/stack/gateway.sh up). Author: kejiqing
set -euo pipefail

LIB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PODMAN_DIR="$(cd "${LIB_DIR}/.." && pwd)"
ROOT_DIR="$(cd "${PODMAN_DIR}/../.." && pwd)"
ROOT_ENV="${ROOT_DIR}/.env"

if [[ ! -f "${ROOT_ENV}" ]]; then
  echo "missing ${ROOT_ENV}" >&2
  echo "copy ${ROOT_DIR}/.env.example to ${ROOT_ENV} and edit" >&2
  exit 1
fi

set -a
# shellcheck disable=SC1090
source "${ROOT_ENV}"
set +a

# shellcheck source=/dev/null
source "${LIB_DIR}/compose-include.sh"
claw_export_llm_runtime_layout "${PODMAN_DIR}"

# shellcheck source=/dev/null
source "${LIB_DIR}/claude-tap-local.sh"
claw_claude_tap_start "${PODMAN_DIR}" "${ROOT_DIR}"

"${PODMAN_DIR}/lib/sync-worker-openai-env.sh"

echo "claude-tap: proxy http://127.0.0.1:${CLAUDE_TAP_PORT:-8080} live http://127.0.0.1:${CLAUDE_TAP_LIVE_PORT:-3000}"
echo "note: .env OPENAI_BASE_URL refreshed for tap; recreate gateway/workers if already running:"
echo "  ./deploy/stack/gateway.sh up"

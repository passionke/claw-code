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
source "${LIB_DIR}/env-profile.sh"
claw_apply_deploy_profile || exit 1

# shellcheck source=/dev/null
source "${LIB_DIR}/compose-include.sh"
claw_export_llm_runtime_layout "${PODMAN_DIR}"

# shellcheck source=/dev/null
source "${LIB_DIR}/pool-health.sh"
# shellcheck source=/dev/null
source "${LIB_DIR}/bootstrap-runtime.sh"

claw_wait_gateway_http_ready 30
if ! claw_gateway_has_active_llm; then
  if ! claw_bootstrap_llm_from_env; then
    echo "error: claude-tap requires active LLM in PostgreSQL (Admin → 全局推理 Apply first)" >&2
    echo "hint: curl -fsS http://127.0.0.1:${GATEWAY_HOST_PORT:-18088}/v1/gateway/global-settings | python3 -m json.tool" >&2
    exit 1
  fi
fi

claw_claude_tap_up_and_register "${PODMAN_DIR}" "${ROOT_DIR}"

if [[ -n "${CLAUDE_TAP_DOCKER_NETWORK:-}" ]]; then
  _tap_admin_host="$(claw_claude_tap_admin_host)"
  echo "claude-tap: docker network $(claw_claude_tap_compose_network_name) — Admin clawTap host=${_tap_admin_host} proxyPort=${CLAUDE_TAP_PORT:-8080} livePort=${CLAUDE_TAP_LIVE_PORT:-3000}"
else
  echo "claude-tap: proxy http://127.0.0.1:${CLAUDE_TAP_PORT:-8080} live http://127.0.0.1:${CLAUDE_TAP_LIVE_PORT:-3000}"
fi
echo "worker runtime env: ${PODMAN_DIR}/.claw-worker-runtime.env (apply with: ./deploy/stack/gateway.sh up)"

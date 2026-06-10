#!/usr/bin/env bash
# Start second logical cluster node on same CI host (gateway-only compose + pool instance). Author: kejiqing
set -euo pipefail

LIB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PODMAN_DIR="$(cd "${LIB_DIR}/.." && pwd)"
REPO_ROOT="$(cd "${PODMAN_DIR}/../.." && pwd)"
ENV_FILE="${1:?env_file}"

if [[ ! -f "${ENV_FILE}" ]]; then
  echo "error: missing ${ENV_FILE}" >&2
  exit 1
fi

set -a
# shellcheck disable=SC1090
source "${ENV_FILE}"
set +a

# shellcheck disable=SC1091
source "${LIB_DIR}/compose-include.sh"
# shellcheck disable=SC1091
source "${LIB_DIR}/pool-health.sh"
# shellcheck disable=SC1091
source "${LIB_DIR}/bootstrap-runtime.sh"
# shellcheck disable=SC1091
source "${LIB_DIR}/claw-pool-registry-env.sh"
# shellcheck disable=SC1091
source "${LIB_DIR}/release-images.sh"
# shellcheck disable=SC1091
source "${LIB_DIR}/fix-session-ownership.sh"

echo "==> CI cluster node B: COMPOSE_PROJECT_NAME=${COMPOSE_PROJECT_NAME} GATEWAY_HOST_PORT=${GATEWAY_HOST_PORT} pool=${CLAW_POOL_ID}" >&2

claw_export_pool_registry_env "$(claw_pool_rpc_root "${PODMAN_DIR}")"
claw_podman_export_pool_workspace "${PODMAN_DIR}"
claw_podman_load_compose_args "${PODMAN_DIR}" "${ENV_FILE}"
claw_reapply_pool_image_pins "${PODMAN_DIR}"

claw_fix_session_workspace_ownership "${CLAW_POOL_WORK_ROOT_BIND_SRC}" || true

# gateway_up → pg_ensure + probe, then create/connect/start (no migrate on node B). kejiqing
claw_compose_gateway_up "${PODMAN_DIR}" "${ENV_FILE}" --force-recreate

claw_wait_gateway_http_ready 90 || {
  echo "error: node B gateway HTTP not ready on :${GATEWAY_HOST_PORT}" >&2
  exit 1
}

if claw_pool_daemon_on_host; then
  export CLAW_POOL_UP_ENV_FILE="${ENV_FILE}"
  set -a
  # shellcheck disable=SC1090
  source "${ENV_FILE}"
  set +a
  "${LIB_DIR}/pool-daemon-up.sh" --restart
  claw_assert_host_pool_rpc_ready "$(claw_pool_rpc_root "${PODMAN_DIR}")" || {
    echo "error: node B pool RPC not ready" >&2
    exit 1
  }
fi

echo "==> CI cluster node B up ok (gateway :${GATEWAY_HOST_PORT}, pool :${CLAW_POOL_HTTP_PORT:-9964})" >&2

#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
ENV_FILE="${REPO_ROOT}/.env"

if [[ ! -f "${ENV_FILE}" ]]; then
  echo "missing ${ENV_FILE}" >&2
  echo "copy ${REPO_ROOT}/.env.example to ${ENV_FILE} and edit" >&2
  exit 1
fi

# Compose bind-mounts repo-root `.claw.json`. Never overwrite an existing file — only create `{}` if missing. kejiqing
CLAW_JSON="${REPO_ROOT}/.claw.json"
if [[ ! -f "${CLAW_JSON}" ]]; then
  echo "note: ${CLAW_JSON} missing; creating empty {} stub (existing files are never touched)." >&2
  printf '%s\n' '{}' > "${CLAW_JSON}"
fi

# shellcheck disable=SC1090
source "${SCRIPT_DIR}/compose-include.sh"
claw_podman_export_pool_workspace "${SCRIPT_DIR}"
claw_podman_load_compose_args "${SCRIPT_DIR}" "${ENV_FILE}"

# Host pool daemon must listen before the gateway connects on first solve. kejiqing
set -a
# shellcheck disable=SC1090
source "${ENV_FILE}"
set +a
if [[ "${CLAW_SOLVE_ISOLATION:-podman_pool}" != "inprocess" ]] && [[ "${CLAW_POOL_HOST_DAEMON:-1}" == "1" ]]; then
  "${SCRIPT_DIR}/pool-daemon-up.sh" "${SCRIPT_DIR}" "${REPO_ROOT}"
fi

# Recreate so env_file changes (e.g. .claw-pool-workspace.env) apply; plain `up -d` can leave stale env. kejiqing
claw_compose --env-file "${ENV_FILE}" "${CLAW_PODMAN_COMPOSE_ARGS[@]}" up -d --force-recreate
echo "Services started."

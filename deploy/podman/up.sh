#!/usr/bin/env bash
# Gateway compose entrypoint. Human config: repo root `.env` only. Generated files under deploy/podman/ are overwritten — do not edit. kejiqing
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
if ! claw_parse_up_release_args "$@"; then
  rc=$?
  if [[ "${rc}" == 2 ]]; then
    exit 0
  fi
  exit "${rc}"
fi

set -a
# shellcheck disable=SC1090
source "${ENV_FILE}"
set +a

if [[ -n "${CLAW_IMAGE_RELEASE_TAG:-}" ]]; then
  claw_apply_release_image_tag "${CLAW_IMAGE_RELEASE_TAG}"
  claw_write_release_pin_env "${SCRIPT_DIR}"
  rt="$(claw_container_runtime_cli)"
  echo "pull ${GATEWAY_IMAGE} …" >&2
  "${rt}" pull "${GATEWAY_IMAGE}"
  case "${CLAW_SOLVE_ISOLATION:-podman_pool}" in
    docker_pool)
      echo "pull ${CLAW_DOCKER_IMAGE} …" >&2
      "${rt}" pull "${CLAW_DOCKER_IMAGE}"
      ;;
    *)
      echo "pull ${CLAW_PODMAN_IMAGE} …" >&2
      "${rt}" pull "${CLAW_PODMAN_IMAGE}"
      ;;
  esac
fi

claw_podman_export_pool_workspace "${SCRIPT_DIR}"
claw_podman_load_compose_args "${SCRIPT_DIR}" "${ENV_FILE}"

cleanup_stale_gateway_workers() {
  local rt ids
  rt="$(claw_container_runtime_cli)"
  ids="$("${rt}" ps -aq --filter name='^claw-gw-' || true)"
  if [[ -z "${ids//[$'\t\r\n ']}" ]]; then
    return 0
  fi
  echo "Removing stale gateway workers before startup..."
  # Keep startup deterministic: stale workers from previous runs can survive compose recreate.
  # Explicitly removing only `claw-gw-*` avoids touching other project containers. Author: kejiqing
  "${rt}" rm -f ${ids}
}

# Host pool daemon must listen before the gateway connects on first solve. kejiqing
set -a
# shellcheck disable=SC1090
source "${ENV_FILE}"
set +a
if [[ "${CLAW_SOLVE_ISOLATION:-podman_pool}" != "inprocess" ]] && [[ "${CLAW_POOL_HOST_DAEMON:-1}" == "1" ]]; then
  "${SCRIPT_DIR}/pool-daemon-up.sh" "${SCRIPT_DIR}" "${REPO_ROOT}"
fi

cleanup_stale_gateway_workers

# Recreate so env_file changes (e.g. .claw-pool-workspace.env) apply; plain `up -d` can leave stale env. kejiqing
claw_compose_with_root_env "${SCRIPT_DIR}" "${ENV_FILE}" "${CLAW_PODMAN_COMPOSE_ARGS[@]}" up -d --force-recreate
echo "Services started."

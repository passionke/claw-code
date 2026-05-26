#!/usr/bin/env bash
# Gateway compose entrypoint. Human config: repo root `.env` only. Generated files under deploy/stack/ are overwritten — do not edit. kejiqing
set -euo pipefail

LIB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PODMAN_DIR="$(cd "${LIB_DIR}/.." && pwd)"
REPO_ROOT="$(cd "${PODMAN_DIR}/../.." && pwd)"
ENV_FILE="${REPO_ROOT}/.env"

# shellcheck disable=SC1091
source "${LIB_DIR}/nuclear-pool-reset.sh"

if [[ ! -f "${ENV_FILE}" ]]; then
  echo "missing ${ENV_FILE}" >&2
  echo "copy ${REPO_ROOT}/.env.example to ${ENV_FILE} and edit" >&2
  exit 1
fi

# Compose auto-loads `.env` next to the compose file; that overrides repo-root `--env-file` / release pins.
if [[ -f "${PODMAN_DIR}/.env" ]]; then
  echo "error: ${PODMAN_DIR}/.env must not exist (Compose implicit env_file). Move keys to ${ENV_FILE} and rm ${PODMAN_DIR}/.env" >&2
  echo "see docs/env-files.md" >&2
  exit 1
fi

# Compose bind-mounts repo-root `.claw.json`. Never overwrite an existing file — only create `{}` if missing. kejiqing
CLAW_JSON="${REPO_ROOT}/.claw.json"
if [[ ! -f "${CLAW_JSON}" ]]; then
  echo "note: ${CLAW_JSON} missing; creating empty {} stub (existing files are never touched)." >&2
  printf '%s\n' '{}' > "${CLAW_JSON}"
fi

# shellcheck disable=SC1090
source "${PODMAN_DIR}/lib/compose-include.sh"
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

# shellcheck source=claw-pool-registry-env.sh
source "${LIB_DIR}/claw-pool-registry-env.sh"
claw_export_pool_registry_env "${PODMAN_DIR}/.claw-pool-rpc"

claw_podman_export_pool_workspace "${PODMAN_DIR}"
claw_ensure_worker_llm_wiring "${PODMAN_DIR}"
claw_export_llm_runtime_layout "${PODMAN_DIR}"
claw_podman_load_compose_args "${PODMAN_DIR}" "${ENV_FILE}"
# shellcheck disable=SC1091
source "${LIB_DIR}/preflight.sh"
claw_deploy_preflight "${PODMAN_DIR}"

# load_compose_args re-sources .env and resets GATEWAY_IMAGE to :local; re-pin after. kejiqing
if [[ -n "${CLAW_IMAGE_RELEASE_TAG:-}" ]]; then
  claw_reapply_pool_image_pins "${PODMAN_DIR}"
fi

# Release up: compose down + kill pool + delete every worker, then pull fresh images. kejiqing
if [[ -n "${CLAW_IMAGE_RELEASE_TAG:-}" ]]; then
  echo "==> release ${CLAW_IMAGE_RELEASE_TAG}: compose down + nuclear pool reset" >&2
  echo "    gateway=${GATEWAY_IMAGE} worker=${CLAW_DOCKER_IMAGE:-${CLAW_PODMAN_IMAGE:-unset}}" >&2
  claw_compose_gateway_down "${PODMAN_DIR}" "${ENV_FILE}" 2>/dev/null || true
  claw_nuclear_pool_reset "${PODMAN_DIR}"
  # Align bind-mount trees once per release (legacy root-owned sessions/logs). kejiqing
  # shellcheck disable=SC1091
  source "${LIB_DIR}/fix-session-ownership.sh"
  claw_prepare_bind_mount_ownership "${PODMAN_DIR}"
  claw_fix_session_workspace_ownership "${CLAW_POOL_WORK_ROOT_BIND_SRC:-${PODMAN_DIR}/claw-workspace}"
  rt="$(claw_container_runtime_cli)"
  echo "pull ${GATEWAY_IMAGE} …" >&2
  "${rt}" pull "${GATEWAY_IMAGE}"
  if [[ -n "${GATEWAY_PLAYGROUND_IMAGE:-}" ]]; then
    echo "pull ${GATEWAY_PLAYGROUND_IMAGE} …" >&2
    "${rt}" pull "${GATEWAY_PLAYGROUND_IMAGE}"
  fi
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

# Pool + compose must not use repo .env :local worker tags when --release or sticky pin is set. kejiqing
claw_reapply_pool_image_pins "${PODMAN_DIR}"
echo "pool daemon worker image: ${CLAW_DOCKER_IMAGE:-${CLAW_PODMAN_IMAGE:-unset}}" >&2
export CLAW_IMAGE_RELEASE_TAG

claw_remove_all_gateway_workers

if claw_pool_daemon_on_host; then
  POOL_BIN="$(claw_ensure_pool_daemon_binary "${PODMAN_DIR}" "${REPO_ROOT}" | tail -n1)"
  export CLAW_POOL_DAEMON_BIN="${POOL_BIN}"
  echo "pool daemon binary: ${POOL_BIN}" >&2
  "${PODMAN_DIR}/lib/pool-daemon-up.sh"
else
  "${PODMAN_DIR}/lib/pool-daemon-down.sh" 2>/dev/null || true
fi

# Recreate gateway container; pool is fresh with pinned worker image. kejiqing
claw_compose_gateway_up "${PODMAN_DIR}" "${ENV_FILE}" --force-recreate
_gw_tag="${GATEWAY_IMAGE##*:}"
if [[ -z "${_gw_tag}" || "${_gw_tag}" == "${GATEWAY_IMAGE}" ]]; then
  _gw_tag="unknown"
fi
echo "Gateway stack started (gateway=${GATEWAY_IMAGE} deployImageTag=${_gw_tag} worker=${CLAW_DOCKER_IMAGE:-${CLAW_PODMAN_IMAGE:-unset}})."
echo "Postgres: use ./deploy/stack/gateway.sh pg-up if not already running."

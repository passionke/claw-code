#!/usr/bin/env bash
# Gateway compose entrypoint. Human config: repo root `.env` only. Generated files under deploy/stack/ are overwritten — do not edit. kejiqing
set -euo pipefail

LIB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PODMAN_DIR="$(cd "${LIB_DIR}/.." && pwd)"
REPO_ROOT="$(cd "${PODMAN_DIR}/../.." && pwd)"
ENV_FILE="${REPO_ROOT}/.env"

# shellcheck disable=SC1091
source "${LIB_DIR}/compose-include.sh"
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

claw_apply_deploy_profile || exit 1

# shellcheck source=claw-pool-registry-env.sh
source "${LIB_DIR}/claw-pool-registry-env.sh"
claw_export_pool_registry_env "$(claw_pool_rpc_root "${PODMAN_DIR}")"

claw_podman_export_pool_workspace "${PODMAN_DIR}"
claw_podman_load_compose_args "${PODMAN_DIR}" "${ENV_FILE}"
# Legacy root-owned ds_* / slots (privileged sidecar) must be fixed before ownership preflight. kejiqing
# shellcheck disable=SC1091
source "${LIB_DIR}/fix-session-ownership.sh"
claw_prepare_bind_mount_ownership "${PODMAN_DIR}"
claw_fix_session_workspace_ownership "${CLAW_POOL_WORK_ROOT_BIND_SRC:-${PODMAN_DIR}/claw-workspace}"
claw_ensure_worker_llm_wiring "${PODMAN_DIR}"
claw_export_llm_runtime_layout "${PODMAN_DIR}"
claw_podman_load_compose_args "${PODMAN_DIR}" "${ENV_FILE}"
# --release sets CLAW_IMAGE_RELEASE_TAG before .env; apply pins before validate. kejiqing
# shellcheck disable=SC1091
source "${LIB_DIR}/release-images.sh"
claw_reapply_pool_image_pins "${PODMAN_DIR}"
# shellcheck disable=SC1091
source "${LIB_DIR}/preflight.sh"
claw_deploy_preflight "${PODMAN_DIR}"
claw_validate_deploy_profile || exit 1

# shellcheck disable=SC1091
source "${LIB_DIR}/claude-tap-local.sh"

# Postgres: ensure running when URL uses compose service / loopback; skip for external PG. kejiqing
if claw_compose_uses_local_postgres; then
  pg="$(claw_compose_pg_service)"
  claw_compose_pg_ensure "${PODMAN_DIR}" "${ENV_FILE}"
  claw_compose_pg_wait_healthy
  echo "Postgres ready (${pg}, host port ${CLAW_GATEWAY_PG_HOST_PORT:-5433})" >&2
fi

# load_compose_args re-sources .env and resets GATEWAY_IMAGE to :local; re-pin after. kejiqing
if [[ -n "${CLAW_IMAGE_RELEASE_TAG:-}" ]]; then
  claw_reapply_pool_image_pins "${PODMAN_DIR}"
fi

# Release up: tap down + compose down + delete legacy workers, then pull fresh images. kejiqing
if [[ -n "${CLAW_IMAGE_RELEASE_TAG:-}" ]]; then
  echo "==> release ${CLAW_IMAGE_RELEASE_TAG}: compose down + nuclear pool reset" >&2
  echo "    gateway=${GATEWAY_IMAGE} worker=${CLAW_DOCKER_IMAGE:-${CLAW_PODMAN_IMAGE:-unset}}" >&2
  if claw_stack_manages_local_claude_tap; then
    echo "==> release: claude-tap down" >&2
    claw_claude_tap_stop "${PODMAN_DIR}"
  fi
  claw_compose_gateway_down "${PODMAN_DIR}" "${ENV_FILE}" 2>/dev/null || true
  claw_nuclear_pool_reset "${PODMAN_DIR}"
  claw_fix_session_workspace_ownership "${CLAW_POOL_WORK_ROOT_BIND_SRC:-${PODMAN_DIR}/claw-workspace}"
  rt="$(claw_container_runtime_cli)"
  claw_release_pull_image_if_needed "${rt}" "${GATEWAY_IMAGE}"
  if [[ -n "${GATEWAY_PLAYGROUND_IMAGE:-}" ]]; then
    claw_release_pull_image_if_needed "${rt}" "${GATEWAY_PLAYGROUND_IMAGE}"
  fi
  case "${CLAW_SOLVE_ISOLATION:-podman_pool}" in
    docker_pool)
      claw_release_pull_image_if_needed "${rt}" "${CLAW_DOCKER_IMAGE}"
      ;;
    *)
      claw_release_pull_image_if_needed "${rt}" "${CLAW_PODMAN_IMAGE}"
      ;;
  esac
  if [[ -n "${CLAW_RELAXED_PODMAN_IMAGE:-}" ]]; then
    claw_release_pull_image_if_needed "${rt}" "${CLAW_RELAXED_PODMAN_IMAGE}"
  fi
  if claw_stack_manages_local_claude_tap; then
    tap_img="${CLAUDE_TAP_IMAGE:-$(claw_default_claude_tap_image)}"
    claw_release_pull_image_if_needed "${rt}" "${tap_img}"
  fi
fi

# Pool + compose must not use repo .env :local worker tags when --release or sticky pin is set. kejiqing
claw_reapply_pool_image_pins "${PODMAN_DIR}"
echo "pool daemon worker image: ${CLAW_DOCKER_IMAGE:-${CLAW_PODMAN_IMAGE:-unset}} (relaxed=${CLAW_RELAXED_PODMAN_IMAGE:-unset})" >&2
export CLAW_IMAGE_RELEASE_TAG

POOL_RPC_DIR="$(claw_pool_rpc_root "${PODMAN_DIR}")"
# Host claw-pool-daemon removed; FC-only. kejiqing

# Recreate gateway only. kejiqing
claw_compose_gateway_up "${PODMAN_DIR}" "${ENV_FILE}" --force-recreate

# Wait for gateway HTTP (and PG migrate) before pool-daemon: both used to call migrate() and deadlock on CI. kejiqing
# shellcheck disable=SC1091
source "${LIB_DIR}/bootstrap-runtime.sh"
claw_wait_gateway_http_ready 60 || exit 1

if ! claw_interactive_backend_is_fc; then
  echo "error: CLAW_INTERACTIVE_BACKEND must be fc (local podman claw-sandbox pool removed)" >&2
  exit 1
fi
echo "FC interactive backend — solve/OVS on e2b sandboxes" >&2

# Default project ds_1 (project_config + workspace init) on every up. kejiqing
# Re-chown after compose up: legacy root-owned ds_* breaks POST /v1/projects on CI runners. kejiqing
claw_prepare_bind_mount_ownership "${PODMAN_DIR}"
claw_fix_session_workspace_ownership "${CLAW_POOL_WORK_ROOT_BIND_SRC:-${PODMAN_DIR}/claw-workspace}" || {
  echo "error: workspace ownership fix failed before ds bootstrap (try gateway.sh fix-workspace)" >&2
  exit 1
}
claw_ensure_default_project_ds "${CLAW_BOOTSTRAP_DS_ID:-1}" || {
  echo "error: default project ds bootstrap failed (POST /v1/projects + /v1/init)" >&2
  exit 1
}
claw_materialize_ovs_workspace_projects || true

# Pool-scoped claude-tap (single sidecar; NAS traces) + Admin register. kejiqing
if claw_stack_manages_local_claude_tap; then
  claw_bootstrap_pool_tap_runtime "${PODMAN_DIR}" "${REPO_ROOT}" || {
    echo "error: pool tap bootstrap failed (LLM / clawTap register)" >&2
    exit 1
  }
fi

_gw_tag="${GATEWAY_IMAGE##*:}"
if [[ -z "${_gw_tag}" || "${_gw_tag}" == "${GATEWAY_IMAGE}" ]]; then
  _gw_tag="unknown"
fi
echo "Gateway stack started (gateway=${GATEWAY_IMAGE} deployImageTag=${_gw_tag} worker=${CLAW_DOCKER_IMAGE:-${CLAW_PODMAN_IMAGE:-unset}})."

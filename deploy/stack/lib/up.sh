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
claw_podman_load_compose_args "${PODMAN_DIR}" "${ENV_FILE}"
# shellcheck disable=SC1091
source "${LIB_DIR}/preflight.sh"
claw_deploy_preflight "${PODMAN_DIR}"

# load_compose_args re-sources .env and resets GATEWAY_IMAGE to :local; re-pin after. kejiqing
if [[ -n "${CLAW_IMAGE_RELEASE_TAG:-}" ]]; then
  claw_apply_release_image_tag "${CLAW_IMAGE_RELEASE_TAG}"
  claw_write_release_pin_env "${PODMAN_DIR}"
fi

# Release up: compose down + kill pool + delete every worker, then pull fresh images. kejiqing
if [[ -n "${CLAW_IMAGE_RELEASE_TAG:-}" ]]; then
  echo "==> release ${CLAW_IMAGE_RELEASE_TAG}: compose down + nuclear pool reset" >&2
  echo "    gateway=${GATEWAY_IMAGE} worker=${CLAW_DOCKER_IMAGE:-${CLAW_PODMAN_IMAGE:-unset}}" >&2
  claw_compose_gateway_down "${PODMAN_DIR}" "${ENV_FILE}" 2>/dev/null || true
  claw_nuclear_pool_reset "${PODMAN_DIR}"
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

# Pin images for pool daemon (never re-source repo .env — it has :local and overwrites release). kejiqing
if [[ -f "${PODMAN_DIR}/.claw-image-release.env" ]]; then
  set -a
  # shellcheck disable=SC1090
  source "${PODMAN_DIR}/.claw-image-release.env"
  set +a
elif [[ -n "${CLAW_IMAGE_RELEASE_TAG:-}" ]]; then
  claw_apply_release_image_tag "${CLAW_IMAGE_RELEASE_TAG}"
fi
echo "pool daemon worker image: ${CLAW_DOCKER_IMAGE:-${CLAW_PODMAN_IMAGE:-unset}}" >&2

claw_remove_all_gateway_workers

if claw_pool_daemon_on_host; then
  POOL_BIN="${CLAW_POOL_DAEMON_BIN:-${REPO_ROOT}/rust/target/release/claw-pool-daemon}"
  # macOS host daemon must match repo sources — never reuse a stale target/release binary. kejiqing
  if [[ "$(uname -s)" == Darwin ]] || [[ "${CLAW_POOL_REBUILD_DAEMON:-0}" == 1 ]]; then
    echo "==> building host claw-pool-daemon (release, required on macOS / CLAW_POOL_REBUILD_DAEMON=1)" >&2
    (cd "${REPO_ROOT}/rust" && cargo build --release -p http-gateway-rs --bin claw-pool-daemon)
    POOL_BIN="${REPO_ROOT}/rust/target/release/claw-pool-daemon"
  elif [[ ! -x "${POOL_BIN}" ]]; then
    if [[ -x "${REPO_ROOT}/deploy/stack/.linux-artifacts/release/claw-pool-daemon" ]]; then
      POOL_BIN="${REPO_ROOT}/deploy/stack/.linux-artifacts/release/claw-pool-daemon"
    else
      echo "==> building host claw-pool-daemon (missing binary)" >&2
      (cd "${REPO_ROOT}/rust" && cargo build --release -p http-gateway-rs --bin claw-pool-daemon)
      POOL_BIN="${REPO_ROOT}/rust/target/release/claw-pool-daemon"
    fi
  fi
  export CLAW_POOL_DAEMON_BIN="${POOL_BIN}"
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

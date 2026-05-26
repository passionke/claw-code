#!/usr/bin/env bash
# Session dirs created by gateway-rs (root) must be uid 1000 for pool workers. Author: kejiqing
set -euo pipefail

LIB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck disable=SC1091
source "${LIB_DIR}/compose-include.sh"

# Logs + workspace roots must be uid 1000 before gateway-rs runs as non-root. kejiqing
claw_prepare_bind_mount_ownership() {
  local podman_dir="${1:?}"
  local uid="${CLAW_WORKER_UID:-1000}"
  local gid="${CLAW_WORKER_GID:-1000}"
  local dir

  mkdir -p "${podman_dir}/claw-logs" "${podman_dir}/claw-workspace"
  for dir in "${podman_dir}/claw-logs" "${podman_dir}/claw-workspace"; do
    [[ -d "${dir}" ]] || continue
    chown -R "${uid}:${gid}" "${dir}" 2>/dev/null || sudo -n chown -R "${uid}:${gid}" "${dir}" 2>/dev/null || true
  done
}

claw_fix_session_workspace_ownership() {
  local root="${1:-${CLAW_POOL_WORK_ROOT_BIND_SRC:-}}"
  local uid="${CLAW_WORKER_UID:-1000}"
  local gid="${CLAW_WORKER_GID:-1000}"
  local rt image ds

  if [[ -z "${root}" || ! -d "${root}" ]]; then
    return 0
  fi

  rt="$(claw_container_runtime_cli)"
  image="${CLAW_CHOWN_RUNNER_IMAGE:-docker.1ms.run/library/alpine:3.20}"

  shopt -s nullglob
  for ds in "${root}"/ds_*/sessions/*; do
    [[ -d "${ds}" ]] || continue
    if [[ "$(stat -c '%u' "${ds}" 2>/dev/null || echo x)" == "${uid}" ]]; then
      continue
    fi
    echo "==> fix session ownership ${ds} -> ${uid}:${gid}" >&2
    if sudo -n chown -R "${uid}:${gid}" "${ds}" 2>/dev/null; then
      continue
    fi
    if chown -R "${uid}:${gid}" "${ds}" 2>/dev/null; then
      continue
    fi
    "${rt}" run --rm -v "${ds}:/mnt:rw" --user root "${image}" \
      chown -R "${uid}:${gid}" /mnt
  done
}

if [[ "${BASH_SOURCE[0]}" == "${0}" ]]; then
  PODMAN_DIR="$(cd "${LIB_DIR}/.." && pwd)"
  REPO_ROOT="$(cd "${PODMAN_DIR}/../.." && pwd)"
  if [[ -f "${REPO_ROOT}/.env" ]]; then
    set -a
    # shellcheck disable=SC1090
    source "${REPO_ROOT}/.env"
    set +a
  fi
  claw_podman_export_pool_workspace "${PODMAN_DIR}"
  claw_fix_session_workspace_ownership "${CLAW_POOL_WORK_ROOT_BIND_SRC:-}"
fi

#!/usr/bin/env bash
# Bind-mount ownership: Linux uid 1000 (pool worker); macOS Podman virtiofs uses **host uid**. Author: kejiqing
set -euo pipefail

LIB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck disable=SC1091
source "${LIB_DIR}/compose-include.sh"

# Linux production: 1000:1000 matches worker USER claw. Darwin: host uid or virtiofs EACCES (gateway panic). kejiqing
claw_bind_mount_owner_uid() {
  if [[ "$(uname -s)" == Darwin ]]; then
    id -u
  else
    printf '%s' "${CLAW_WORKER_UID:-1000}"
  fi
}

claw_bind_mount_owner_gid() {
  if [[ "$(uname -s)" == Darwin ]]; then
    id -g
  else
    printf '%s' "${CLAW_WORKER_GID:-1000}"
  fi
}

# Linux: stat -c; macOS/BSD: stat -f. Author: kejiqing
claw_path_uid() {
  local path="${1:?}"
  local u
  u="$(stat -c '%u' "${path}" 2>/dev/null)" && {
    printf '%s' "${u}"
    return 0
  }
  stat -f '%u' "${path}" 2>/dev/null || echo x
}

claw_path_owned_by() {
  local path="${1:?}"
  local uid="${2:?}"
  [[ "$(claw_path_uid "${path}")" == "${uid}" ]]
}

# Pool workers (container uid 1000) still need write on Darwin; host uid dirs get a+rwX. kejiqing
claw_chmod_tree_writable() {
  local path="${1:?}"
  local rt image parent base

  [[ -e "${path}" ]] || return 0
  if chmod -R a+rwX "${path}" 2>/dev/null; then
    return 0
  fi
  rt="$(claw_container_runtime_cli)"
  image="${CLAW_CHOWN_RUNNER_IMAGE:-docker.1ms.run/library/alpine:3.20}"
  parent="$(cd "$(dirname "${path}")" && pwd)"
  base="$(basename "${path}")"
  echo "==> chmod via ${rt} ${image}: ${path} a+rwX (Darwin pool worker)" >&2
  "${rt}" run --rm -v "${parent}:/mnt:rw" --user root "${image}" \
    chmod -R a+rwX "/mnt/${base}" 2>/dev/null || true
}

# chown tree for bind mounts; docker-run fallback when host cannot fix orphan uid. kejiqing
claw_chown_tree_to_worker() {
  local path="${1:?}"
  local uid="${2:-$(claw_bind_mount_owner_uid)}"
  local gid="${3:-$(claw_bind_mount_owner_gid)}"
  local rt image parent base

  [[ -e "${path}" ]] || return 0
  if chown -R "${uid}:${gid}" "${path}" 2>/dev/null; then
    [[ "$(uname -s)" == Darwin ]] && claw_chmod_tree_writable "${path}" || true
    return 0
  fi
  if sudo -n chown -R "${uid}:${gid}" "${path}" 2>/dev/null; then
    [[ "$(uname -s)" == Darwin ]] && claw_chmod_tree_writable "${path}" || true
    return 0
  fi
  if sudo chown -R "${uid}:${gid}" "${path}" 2>/dev/null; then
    [[ "$(uname -s)" == Darwin ]] && claw_chmod_tree_writable "${path}" || true
    return 0
  fi
  rt="$(claw_container_runtime_cli)"
  image="${CLAW_CHOWN_RUNNER_IMAGE:-docker.1ms.run/library/alpine:3.20}"
  parent="$(cd "$(dirname "${path}")" && pwd)"
  base="$(basename "${path}")"
  echo "==> chown via ${rt} ${image}: ${path} -> ${uid}:${gid}" >&2
  "${rt}" run --rm -v "${parent}:/mnt:rw" --user root "${image}" \
    chown -R "${uid}:${gid}" "/mnt/${base}"
  if [[ "$(uname -s)" == Darwin ]]; then
    claw_chmod_tree_writable "${path}"
  fi
}

# Workspace + claw-logs before gateway-rs. Linux: chown 1000; Darwin: mkdir only (:U at compose up). kejiqing
claw_prepare_bind_mount_ownership() {
  local podman_dir="${1:?}"
  local uid gid ws log_dir

  ws="$(claw_stack_workspace_bind_dir "${podman_dir}")"
  mkdir -p "${ws}"

  log_dir="${CLAW_HOST_LOG_DIR:-${podman_dir}/claw-logs}"
  if [[ "${log_dir}" != /* ]]; then
    log_dir="${podman_dir}/${log_dir#./}"
  fi
  mkdir -p "${log_dir}"

  if [[ "$(uname -s)" == Darwin ]]; then
    return 0
  fi

  uid="$(claw_bind_mount_owner_uid)"
  gid="$(claw_bind_mount_owner_gid)"
  claw_chown_tree_to_worker "${ws}" "${uid}" "${gid}"
  claw_chown_tree_to_worker "${log_dir}" "${uid}" "${gid}"
}

claw_fix_session_workspace_ownership() {
  local root="${1:-${CLAW_POOL_WORK_ROOT_BIND_SRC:-}}"
  local uid gid rt image ds

  if [[ -z "${root}" || ! -d "${root}" ]]; then
    return 0
  fi

  if [[ "$(uname -s)" == Darwin ]]; then
    # :U + gateway user=host uid; only strip root-owned paths (sidecar). kejiqing
    uid="$(claw_bind_mount_owner_uid)"
    shopt -s nullglob
    for ds in "${root}"/ds_* "${root}"/proj_*; do
      [[ -d "${ds}" ]] || continue
      if [[ "$(claw_path_uid "${ds}")" == "0" ]]; then
        echo "==> fix root-owned ${ds} -> ${uid}" >&2
        claw_chown_tree_to_worker "${ds}" "${uid}" "$(claw_bind_mount_owner_gid)" || return 1
      fi
    done
    return 0
  fi

  uid="$(claw_bind_mount_owner_uid)"
  gid="$(claw_bind_mount_owner_gid)"
  local rt image ds

  if [[ -z "${root}" || ! -d "${root}" ]]; then
    return 0
  fi

  rt="$(claw_container_runtime_cli)"
  image="${CLAW_CHOWN_RUNNER_IMAGE:-docker.1ms.run/library/alpine:3.20}"

  shopt -s nullglob
  for ds in "${root}"/ds_* "${root}"/proj_*; do
    [[ -d "${ds}" ]] || continue
    if ! claw_path_owned_by "${ds}" "${uid}"; then
      echo "==> fix project workspace ownership ${ds} -> ${uid}:${gid}" >&2
      claw_chown_tree_to_worker "${ds}" "${uid}" "${gid}" || return 1
    fi
  done

  for ds in "${root}"/ds_*/sessions/* "${root}"/proj_*/sessions/*; do
    [[ -d "${ds}" ]] || continue
    if claw_path_owned_by "${ds}" "${uid}"; then
      continue
    fi
    echo "==> fix session ownership ${ds} -> ${uid}:${gid}" >&2
    claw_chown_tree_to_worker "${ds}" "${uid}" "${gid}" || return 1
  done

  # Slot guests are recreated by pool; chown so a later preflight on ds_* paths stays clean. kejiqing
  local slot_root="${root}/.claw-pool-slot"
  if [[ -d "${slot_root}" ]] && ! claw_path_owned_by "${slot_root}" "${uid}"; then
    echo "==> fix pool slot ownership ${slot_root} -> ${uid}:${gid}" >&2
    claw_chown_tree_to_worker "${slot_root}" "${uid}" "${gid}" || return 1
  fi
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

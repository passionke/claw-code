#!/usr/bin/env bash
# One-shot host checks before `gateway.sh up` (rootless podman / legacy docker-compose). Author: kejiqing
set -euo pipefail

claw_deploy_preflight() {
  local podman_dir="${1:?}"
  # shellcheck disable=SC1091
  source "${podman_dir}/lib/compose-include.sh"

  echo "==> deploy preflight" >&2
  local rt sock
  rt="$(claw_container_runtime_cli)" || return 1
  echo "    runtime=${rt}" >&2

  if ! "${rt}" info >/dev/null 2>&1; then
    echo "error: ${rt} info failed" >&2
    return 1
  fi

  sock="$(claw_container_socket_path)" || return 1
  export CLAW_CONTAINER_SOCKET="${sock}"
  export DOCKER_HOST="unix://${sock}"
  if [[ ! -S "${sock}" ]] || [[ ! -r "${sock}" || ! -w "${sock}" ]]; then
    echo "error: cannot use container socket: ${sock}" >&2
    if [[ "${rt}" == docker ]]; then
      echo "hint: sudo systemctl start docker  OR  ./deploy/stack/gateway.sh install-docker" >&2
      echo "      production uses CLAW_CONTAINER_RUNTIME=docker (no CLAW_CONTAINER_SOCKET)" >&2
    else
      echo "hint: podman machine start (macOS) or set CLAW_CONTAINER_SOCKET (Linux rootless)" >&2
    fi
    return 1
  fi
  echo "    socket=${sock}" >&2

  export COMPOSE_PROJECT_NAME="${COMPOSE_PROJECT_NAME:-claw}"
  # Docker Compose v2 owns project networks; manual `network create` breaks labels.
  if [[ "${rt}" == podman ]]; then
    claw_network_ensure "${rt}" "${COMPOSE_PROJECT_NAME}_default"
    claw_network_ensure "${rt}" "stack_default"
  fi

  if claw_compose_uses_local_postgres; then
    local pg="${CLAW_GATEWAY_PG_IMAGE:-docker.io/library/postgres:17-alpine}"
    if ! "${rt}" image exists "${pg}" >/dev/null 2>&1; then
      echo "    pull ${pg} …" >&2
      "${rt}" pull "${pg}"
    fi
  else
    echo "    postgres: external (${CLAW_GATEWAY_DATABASE_URL%%@*}@…); skip local image pull" >&2
  fi

  if claw_pool_uses_remote; then
    echo "    workspace: skip ownership preflight (remote pool; workers not on this host)" >&2
    mkdir -p "$(claw_stack_workspace_bind_dir "${podman_dir}")" 2>/dev/null || true
  elif claw_compose_nas_volume_enabled; then
    echo "    workspace: NFS compose volume (Gateway/OVS direct NAS; skip host bind preflight)" >&2
    mkdir -p "$(claw_stack_workspace_bind_dir "${podman_dir}")" 2>/dev/null || true
  else
    claw_workspace_ownership_preflight "${podman_dir}" || return 1
  fi

  if claw_pool_uses_remote; then
    [[ -n "${CLAW_POOL_ID:-}" ]] || {
      echo "error: CLAW_POOL_ID required with CLAW_POOL_REMOTE_BASE" >&2
      return 1
    }
    echo "    pool: remote ${CLAW_POOL_REMOTE_BASE} (pool_id=${CLAW_POOL_ID})" >&2
  else
    claw_pool_daemon_on_host || return 1
  fi

  echo "==> preflight ok (compose project=${COMPOSE_PROJECT_NAME})" >&2
}

claw_workspace_ownership_preflight() {
  local podman_dir="${1:?}"
  local root="${CLAW_POOL_WORK_ROOT_BIND_SRC:-${podman_dir}/claw-workspace}"
  local uid="${CLAW_WORKER_UID:-1000}"
  local gid="${CLAW_WORKER_GID:-1000}"
  local out
  local lib_dir
  lib_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

  if [[ "$(uname -s)" == Darwin ]]; then
    # shellcheck disable=SC1091
    source "${lib_dir}/fix-session-ownership.sh"
    local want_uid want_gid
    want_uid="$(claw_bind_mount_owner_uid)"
    want_gid="$(claw_bind_mount_owner_gid)"
    if [[ -n "${CLAW_WORKER_UID:-}" && "${CLAW_WORKER_UID}" != "${want_uid}" ]]; then
      echo "error: on macOS do not set CLAW_WORKER_UID=${CLAW_WORKER_UID} in .env — env-profile uses host uid ${want_uid} for bind mounts" >&2
      return 1
    fi
    if [[ -n "${CLAW_WORKER_GID:-}" && "${CLAW_WORKER_GID}" != "${want_gid}" ]]; then
      echo "error: on macOS do not set CLAW_WORKER_GID=${CLAW_WORKER_GID} in .env — env-profile uses host gid ${want_gid}" >&2
      return 1
    fi
  fi

  if [[ ! -d "${root}" ]]; then
    echo "error: workspace root missing: ${root}" >&2
    echo "hint: mkdir -p \"${root}\" && sudo chown -R ${uid}:${gid} \"${root}\"" >&2
    echo "manual: deploy/stack/README.md -> 1.3 启动与检查 / 3. 常见问题（短）" >&2
    return 1
  fi

  # Sidecar/privileged pool leaves root-owned ds_*; slots are recreated on up. kejiqing
  # shellcheck disable=SC1091
  source "${lib_dir}/fix-session-ownership.sh"
  rm -rf "${root}/.claw-pool-slot" 2>/dev/null || true
  claw_fix_session_workspace_ownership "${root}"

  # macOS bind mount: host stat uid != 1000 even when container worker (uid 1000) can write. kejiqing
  if [[ "$(uname -s)" == Darwin ]]; then
    out="$(
    python3 - "${root}" <<'PY'
import os, sys
root = sys.argv[1]
errors = []
for name in os.listdir(root):
    if not (name.startswith("ds_") or name.startswith("proj_")):
        continue
    base = os.path.join(root, name)
    for dirpath, dirnames, filenames in os.walk(base):
        for fn in filenames + dirnames:
            path = os.path.join(dirpath, fn)
            try:
                st = os.lstat(path)
            except (FileNotFoundError, PermissionError):
                continue
            if st.st_uid == 0:
                errors.append(path)
if errors:
    print(f"ROOT_OWNED {len(errors)}")
    for p in errors[:10]:
        print(p)
    sys.exit(2)
print("OK macOS")
PY
  )" || true
    if [[ "${out}" == OK* ]]; then
      echo "    workspace ownership ok (macOS bind mount; container worker uid ${uid})" >&2
      return 0
    fi
    if [[ "${out}" == ROOT_OWNED* ]]; then
      echo "error: workspace has root-owned paths under ${root}" >&2
      printf '%s\n' "${out}" >&2
      echo "hint: ./deploy/stack/gateway.sh fix-workspace" >&2
      return 1
    fi
  fi

  out="$(
    python3 - "${root}" "${uid}" "${gid}" <<'PY'
import os
import stat
import sys

root = sys.argv[1]
want_uid = int(sys.argv[2])
want_gid = int(sys.argv[3])
errors = []
checked = 0

def check_path(path):
    global checked
    if "/.claw-pool-slot/" in path or path.rstrip("/").endswith("/.claw-pool-slot"):
        return
    try:
        st = os.lstat(path)
    except FileNotFoundError:
        return
    checked += 1
    if st.st_uid != want_uid or st.st_gid != want_gid:
        mode = stat.filemode(st.st_mode)
        errors.append(f"{path}|uid={st.st_uid}|gid={st.st_gid}|mode={mode}")

try:
    names = os.listdir(root)
except FileNotFoundError:
    names = []
except PermissionError:
    print(f"PERMISSION_DENIED {root}")
    sys.exit(3)
for name in names:
    if not (name.startswith("ds_") or name.startswith("proj_")):
        continue
    base = os.path.join(root, name)
    check_path(base)
    for dirpath, dirnames, filenames in os.walk(base):
        for fn in filenames:
            check_path(os.path.join(dirpath, fn))
        for dn in dirnames:
            check_path(os.path.join(dirpath, dn))

if errors:
    print(f"MISMATCH {len(errors)} {checked}")
    for line in errors[:20]:
        print(line)
    sys.exit(2)

print(f"OK {checked}")
PY
  )" || true

  if [[ "${out}" == MISMATCH* ]]; then
    echo "error: workspace ownership preflight failed: root=${root} expected=${uid}:${gid}" >&2
    printf '%s\n' "${out}" >&2
    echo "hint: ./deploy/stack/gateway.sh fix-workspace  OR  sudo chown -R ${uid}:${gid} \"${root}/ds_\"*" >&2
    echo "manual: deploy/stack/README.md -> 1.3 启动与检查 / 3. 常见问题（短）" >&2
    return 1
  fi

  if [[ "${out}" == PERMISSION_DENIED* ]]; then
    echo "error: cannot read workspace ${root} (permission denied — often root-owned from prior compose)" >&2
    echo "hint: sudo chown -R \$(id -u):\$(id -g) \"${root}\"  OR  ./deploy/stack/gateway.sh fix-workspace" >&2
    return 1
  fi

  if [[ "${out}" != OK* ]]; then
    echo "error: workspace ownership preflight failed unexpectedly: ${out}" >&2
    echo "manual: deploy/stack/README.md -> 1.3 启动与检查 / 3. 常见问题（短）" >&2
    return 1
  fi

  echo "    workspace ownership ok: root=${root} owner=${uid}:${gid}" >&2
}

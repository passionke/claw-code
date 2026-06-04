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
      echo "hint: start Docker; production uses CLAW_CONTAINER_RUNTIME=docker (no CLAW_CONTAINER_SOCKET)" >&2
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

  local pg="${CLAW_GATEWAY_PG_IMAGE:-docker.io/library/postgres:17-alpine}"
  if ! "${rt}" image exists "${pg}" >/dev/null 2>&1; then
    echo "    pull ${pg} …" >&2
    "${rt}" pull "${pg}"
  fi

  claw_workspace_ownership_preflight "${podman_dir}" || return 1

  echo "==> preflight ok (compose project=${COMPOSE_PROJECT_NAME})" >&2
}

claw_workspace_ownership_preflight() {
  local podman_dir="${1:?}"
  local root="${CLAW_POOL_WORK_ROOT_BIND_SRC:-${podman_dir}/claw-workspace}"
  local uid="${CLAW_WORKER_UID:-1000}"
  local gid="${CLAW_WORKER_GID:-1000}"
  local out

  if [[ ! -d "${root}" ]]; then
    echo "error: workspace root missing: ${root}" >&2
    echo "hint: mkdir -p \"${root}\" && sudo chown -R ${uid}:${gid} \"${root}\"" >&2
    echo "manual: deploy/stack/README.md -> 1.3 启动与检查 / 3. 常见问题（短）" >&2
    return 1
  fi

  out="$(
    python3 - "${root}" "${uid}" "${gid}" <<'PY'
import os
import stat
import sys

root = sys.argv[1]
want_uid = int(sys.argv[2])
want_gid = int(sys.argv[3])
# Pool daemon (often privileged root in compose) owns slot mounts; not session uid 1000. kejiqing
POOL_SLOT_BASENAME = ".claw-pool-slot"
errors = []
checked = 0

def check_path(path):
    global checked
    try:
        st = os.lstat(path)
    except FileNotFoundError:
        return
    checked += 1
    if st.st_uid != want_uid or st.st_gid != want_gid:
        mode = stat.filemode(st.st_mode)
        errors.append(f"{path}|uid={st.st_uid}|gid={st.st_gid}|mode={mode}")

check_path(root)
for dirpath, dirnames, filenames in os.walk(root):
    if POOL_SLOT_BASENAME in dirnames:
        dirnames.remove(POOL_SLOT_BASENAME)
    for name in dirnames:
        check_path(os.path.join(dirpath, name))
    for name in filenames:
        check_path(os.path.join(dirpath, name))

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
    echo "hint: sudo chown -R ${uid}:${gid} \"${root}\"" >&2
    echo "manual: deploy/stack/README.md -> 1.3 启动与检查 / 3. 常见问题（短）" >&2
    return 1
  fi

  if [[ "${out}" != OK* ]]; then
    echo "error: workspace ownership preflight failed unexpectedly: ${out}" >&2
    echo "manual: deploy/stack/README.md -> 1.3 启动与检查 / 3. 常见问题（短）" >&2
    return 1
  fi

  echo "    workspace ownership ok: root=${root} owner=${uid}:${gid}" >&2
}

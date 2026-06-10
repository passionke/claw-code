#!/usr/bin/env bash
# Default host pool-daemon path + install from GATEWAY_IMAGE (gateway.sh up). Author: kejiqing

claw_default_pool_daemon_bin() {
  local podman_dir="${1:?}"
  printf '%s\n' "${podman_dir}/.linux-artifacts/release/claw-sandbox"
}

# Local pack-deploy uses claw-gateway-rs:local; release uses */claw-code:<tag>. kejiqing
claw_gateway_image_carries_pool_daemon() {
  case "${1:-}" in
    *claw-code* | *claw-gateway-rs*) return 0 ;;
    *) return 1 ;;
  esac
}

# Resolve executable for host claw-sandbox (pool daemon). Author: kejiqing
claw_ensure_pool_daemon_binary() {
  local podman_dir="${1:?}"
  local repo_root="${2:?}"
  local out
  out="$(claw_default_pool_daemon_bin "${podman_dir}")"
  mkdir -p "$(dirname "${out}")"

  if [[ "$(uname -s)" == Darwin ]]; then
    local mac_bin="${repo_root}/sandbox/target/release/claw-sandbox"
    if [[ "${CLAW_POOL_REBUILD_DAEMON:-0}" == 1 ]] || [[ ! -x "${mac_bin}" ]]; then
      echo "==> building host claw-sandbox (macOS)" >&2
      (cd "${repo_root}/sandbox" && cargo build --release -p claw-sandbox-server)
    else
      echo "==> reusing host claw-sandbox: ${mac_bin}" >&2
    fi
    printf '%s\n' "${mac_bin}"
    return 0
  fi

  if [[ "${CLAW_POOL_REBUILD_DAEMON:-0}" == 1 ]]; then
    echo "==> building host claw-sandbox (CLAW_POOL_REBUILD_DAEMON=1)" >&2
    (cd "${repo_root}/sandbox" && cargo build --release -p claw-sandbox-server)
    cp -f "${repo_root}/sandbox/target/release/claw-sandbox" "${out}"
    printf '%s\n' "${out}"
    return 0
  fi

  local gw="${GATEWAY_IMAGE:-}"
  if claw_gateway_image_carries_pool_daemon "${gw}"; then
    local refresh=0
    if [[ -n "${CLAW_IMAGE_RELEASE_TAG:-}" ]]; then
      refresh=1
    elif [[ ! -x "${out}" ]]; then
      refresh=1
    fi
    if [[ "${refresh}" == 1 ]]; then
      echo "==> install claw-pool-daemon from ${gw}" >&2
      echo "    -> ${out}" >&2
      GATEWAY_IMAGE="${gw}" "${podman_dir}/lib/install-pool-daemon-from-image.sh" "${out}"
    fi
    if [[ -x "${out}" ]]; then
      printf '%s\n' "${out}"
      return 0
    fi
  fi

  # Ignore .env CLAW_POOL_DAEMON_BIN when release deploy installs under .linux-artifacts/. kejiqing
  if [[ -z "${CLAW_IMAGE_RELEASE_TAG:-}" && -n "${CLAW_POOL_DAEMON_BIN:-}" && -x "${CLAW_POOL_DAEMON_BIN}" ]]; then
    printf '%s\n' "${CLAW_POOL_DAEMON_BIN}"
    return 0
  fi

  if [[ -x "${out}" ]]; then
    printf '%s\n' "${out}"
    return 0
  fi

  echo "error: no claw-sandbox (build sandbox/ or set CLAW_POOL_DAEMON_BIN)" >&2
  return 1
}

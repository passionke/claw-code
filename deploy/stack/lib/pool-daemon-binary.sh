#!/usr/bin/env bash
# Default host pool-daemon path + install from GATEWAY_IMAGE (gateway.sh up). Author: kejiqing

claw_default_pool_daemon_bin() {
  local podman_dir="${1:?}"
  printf '%s\n' "${podman_dir}/.linux-artifacts/release/claw-pool-daemon"
}

# Resolve executable for host claw-pool-daemon. On Linux release deploy, always refresh from GATEWAY_IMAGE.
claw_ensure_pool_daemon_binary() {
  local podman_dir="${1:?}"
  local repo_root="${2:?}"
  local out
  out="$(claw_default_pool_daemon_bin "${podman_dir}")"
  mkdir -p "$(dirname "${out}")"

  if [[ "$(uname -s)" == Darwin ]] || [[ "${CLAW_POOL_REBUILD_DAEMON:-0}" == 1 ]]; then
    echo "==> building host claw-pool-daemon (macOS or CLAW_POOL_REBUILD_DAEMON=1)" >&2
    (cd "${repo_root}/rust" && cargo build --release -p http-gateway-rs --bin claw-pool-daemon)
    printf '%s\n' "${repo_root}/rust/target/release/claw-pool-daemon"
    return 0
  fi

  local gw="${GATEWAY_IMAGE:-}"
  if [[ "${gw}" == *claw-code* ]]; then
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

  if [[ -n "${CLAW_POOL_DAEMON_BIN:-}" && -x "${CLAW_POOL_DAEMON_BIN}" ]]; then
    printf '%s\n' "${CLAW_POOL_DAEMON_BIN}"
    return 0
  fi

  if [[ -x "${out}" ]]; then
    printf '%s\n' "${out}"
    return 0
  fi

  echo "error: no claw-pool-daemon (set GATEWAY_IMAGE to claw-code:release-* or build on host)" >&2
  return 1
}

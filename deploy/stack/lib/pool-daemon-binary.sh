#!/usr/bin/env bash
# Default host pool binary path (claw-sandbox). Release: pull image + extract; dev: linux-compile/cargo. Author: kejiqing

claw_default_pool_daemon_bin() {
  local podman_dir="${1:?}"
  printf '%s\n' "${podman_dir}/.linux-artifacts/release/claw-sandbox"
}

claw_pool_daemon_release_deploy_active() {
  [[ -n "${CLAW_IMAGE_RELEASE_TAG:-}" ]] && return 0
  [[ -n "${CLAW_SANDBOX_IMAGE:-}" ]] && return 0
  local gw="${GATEWAY_IMAGE:-}"
  [[ "${gw}" == *"/claw-code:release-"* ]] && return 0
  return 1
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

  # Linux production / release: never host cargo when release pin active or SKIP_BUILD=1.
  if claw_pool_daemon_release_deploy_active || [[ "${CLAW_POOL_DAEMON_SKIP_BUILD:-0}" == "1" ]]; then
    # shellcheck source=install-claw-sandbox-from-release.sh
    source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/install-claw-sandbox-from-release.sh"
    claw_install_claw_sandbox_from_release "${podman_dir}" || return 1
    printf '%s\n' "${out}"
    return 0
  fi

  if [[ -x "${out}" ]] && [[ "${CLAW_POOL_REBUILD_DAEMON:-0}" != "1" ]]; then
    echo "==> reusing host claw-sandbox: ${out}" >&2
    printf '%s\n' "${out}"
    return 0
  fi

  if [[ "${CLAW_POOL_REBUILD_DAEMON:-0}" == 1 ]] || [[ ! -x "${out}" ]]; then
    if [[ -x "${out}" ]]; then
      echo "==> rebuilding host claw-sandbox (CLAW_POOL_REBUILD_DAEMON=1)" >&2
    else
      echo "==> building host claw-sandbox (missing ${out})" >&2
    fi
    (cd "${repo_root}/sandbox" && cargo build --release -p claw-sandbox-server)
    cp -f "${repo_root}/sandbox/target/release/claw-sandbox" "${out}"
    printf '%s\n' "${out}"
    return 0
  fi

  if [[ -n "${CLAW_POOL_DAEMON_BIN:-}" && -x "${CLAW_POOL_DAEMON_BIN}" ]]; then
    printf '%s\n' "${CLAW_POOL_DAEMON_BIN}"
    return 0
  fi

  echo "error: no claw-sandbox (run gateway.sh up --release <tag> or gateway.sh build)" >&2
  return 1
}

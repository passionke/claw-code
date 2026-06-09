#!/usr/bin/env bash
# Linux production: claw-pool-daemon under systemd (root; docker mount). Author: kejiqing
# Dual pool: one unit per profile (strict/relaxed), mirroring launchd labels on macOS.
set -euo pipefail

# strict → claw-pool-daemon-strict.service; legacy empty profile → claw-pool-daemon.service
claw_pool_systemd_unit() {
  local profile="${1:-}"
  if [[ -n "${profile}" ]]; then
    printf '%s' "claw-pool-daemon-${profile}"
  else
    printf '%s' "claw-pool-daemon"
  fi
}

claw_pool_systemd_unit_path() {
  local profile="${1:-}"
  printf '%s' "/etc/systemd/system/$(claw_pool_systemd_unit "${profile}").service"
}

# production Linux host pool (docker_pool needs mount --make-rshared → root).
claw_pool_use_systemd() {
  [[ "$(uname -s)" == "Linux" ]] || return 1
  case "${CLAW_POOL_DAEMON_USE_SYSTEMD:-}" in
    0 | false | no | off) return 1 ;;
    1 | true | yes | on) ;;
  esac
  # shellcheck disable=SC1091
  source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/env-profile.sh"
  [[ "$(claw_deploy_profile_name)" == "production" ]] || return 1
  # Never prompt for password (CI runners / gitlab-runner). kejiqing
  sudo -n true 2>/dev/null
}

claw_pool_sudo() {
  sudo -n "$@" 2>/dev/null
}

claw_pool_systemd_installed() {
  local profile="${1:-}"
  [[ -f "$(claw_pool_systemd_unit_path "${profile}")" ]]
}

claw_pool_systemd_active() {
  local profile="${1:-}"
  claw_pool_sudo systemctl is-active --quiet "$(claw_pool_systemd_unit "${profile}")" 2>/dev/null
}

claw_pool_systemd_write_unit() {
  local rpc_dir="$1" run_sh="$2" repo_root="$3" profile="${4:-}"
  local unit desc
  unit="$(claw_pool_systemd_unit "${profile}")"
  if [[ -n "${profile}" ]]; then
    desc="claw-pool-daemon ${profile} (docker_pool worker pool)"
  else
    desc="claw-pool-daemon (docker_pool worker pool)"
  fi
  claw_pool_sudo tee "$(claw_pool_systemd_unit_path "${profile}")" >/dev/null <<EOF
[Unit]
Description=${desc}
After=network-online.target docker.service
Wants=network-online.target

[Service]
Type=simple
User=root
Group=root
WorkingDirectory=${repo_root}
ExecStart=${run_sh} ${rpc_dir}
Restart=on-failure
RestartSec=3
KillMode=process

[Install]
WantedBy=multi-user.target
EOF
}

# Pre-dual-pool hosts had a single unit; disable so strict+relaxed can coexist. kejiqing
claw_pool_systemd_retire_legacy_unit() {
  local legacy
  legacy="$(claw_pool_systemd_unit "")"
  if claw_pool_systemd_installed "" && claw_pool_sudo systemctl is-enabled --quiet "${legacy}" 2>/dev/null; then
    echo "==> retiring legacy systemd unit ${legacy} (dual-pool migration)" >&2
    claw_pool_sudo systemctl stop "${legacy}" 2>/dev/null || true
    claw_pool_sudo systemctl disable "${legacy}" 2>/dev/null || true
  fi
}

claw_pool_systemd_install_and_restart() {
  local rpc_dir="$1" run_sh="$2" repo_root="$3" profile="${4:-}"
  local unit
  unit="$(claw_pool_systemd_unit "${profile}")"
  if [[ -n "${profile}" ]]; then
    claw_pool_systemd_retire_legacy_unit
  fi
  claw_pool_systemd_write_unit "${rpc_dir}" "${run_sh}" "${repo_root}" "${profile}"
  claw_pool_sudo systemctl daemon-reload
  claw_pool_sudo systemctl enable "${unit}"
  claw_pool_sudo systemctl restart "${unit}"
}

claw_pool_systemd_stop() {
  local profile="${1:-}"
  local unit
  unit="$(claw_pool_systemd_unit "${profile}")"
  if claw_pool_systemd_installed "${profile}"; then
    claw_pool_sudo systemctl stop "${unit}" 2>/dev/null || true
  fi
}

# gitlab-runner has docker but not passwordless sudo; stop host systemd before SIGKILL (Restart=on-failure). kejiqing
claw_pool_systemd_stop_via_docker() {
  local profile="${1:-}" unit rt image lib_dir
  unit="$(claw_pool_systemd_unit "${profile}")"
  [[ -f "$(claw_pool_systemd_unit_path "${profile}")" ]] || return 1
  lib_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
  # shellcheck disable=SC1091
  source "${lib_dir}/compose-include.sh"
  rt="$(claw_container_runtime_cli)" || return 1
  image="${CONTAINER_BASE_REGISTRY:-docker.1ms.run}/library/alpine:3.20"
  echo "==> systemctl stop+disable ${unit} via ${rt} chroot /host" >&2
  "${rt}" run --rm --privileged --pid=host \
    -v /:/host \
    -v /run/systemd:/run/systemd \
    -v /run/systemd/system:/run/systemd/system \
    "${image}" sh -c "
      apk add --no-cache util-linux >/dev/null 2>&1 || true
      chroot /host systemctl stop '${unit}' 2>/dev/null || true
      chroot /host systemctl disable '${unit}' 2>/dev/null || true
      nsenter -t 1 -m -u -i -n -p systemctl stop '${unit}' 2>/dev/null || true
    "
}

claw_pool_systemd_main_pid() {
  local profile="${1:-}"
  claw_pool_sudo systemctl show "$(claw_pool_systemd_unit "${profile}")" -p MainPID --value 2>/dev/null || true
}

# Fail verify when dual-pool dirs exist but legacy single unit still owns strict HTTP. Author: kejiqing
claw_pool_systemd_assert_dual_pool_coherent() {
  local podman_dir="${1:?podman_dir}"
  local strict_dir="${podman_dir}/.claw-pool-rpc/strict"
  local relaxed_dir="${podman_dir}/.claw-pool-rpc/relaxed"
  local strict_port="${CLAW_STRICT_POOL_HTTP_PORT:-9944}"
  local legacy_unit strict_unit

  [[ -d "${strict_dir}" && -d "${relaxed_dir}" ]] || return 0
  claw_pool_use_systemd 2>/dev/null || return 0

  legacy_unit="$(claw_pool_systemd_unit "")"
  strict_unit="$(claw_pool_systemd_unit strict)"

  if claw_pool_systemd_active "" && ! claw_pool_systemd_active strict; then
    echo "VERIFY FAIL: dual-pool layout but legacy ${legacy_unit} is active without ${strict_unit}" >&2
    echo "  fix: ./deploy/stack/gateway.sh pool-up --restart --profile=all" >&2
    echo "  or set CLAW_POOL_DAEMON_USE_SYSTEMD=0 in .env and pool-up again" >&2
    return 1
  fi

  if claw_pool_systemd_active strict && ! curl -fsS --connect-timeout 2 \
    "http://127.0.0.1:${strict_port}/healthz/live-report" >/dev/null 2>&1; then
    echo "VERIFY FAIL: ${strict_unit} active in systemd but 127.0.0.1:${strict_port} not reachable" >&2
    return 1
  fi
  return 0
}

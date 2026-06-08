#!/usr/bin/env bash
# Linux production: claw-pool-daemon under systemd (root; docker mount). Author: kejiqing
set -euo pipefail

claw_pool_systemd_unit() {
  printf '%s' "claw-pool-daemon"
}

claw_pool_systemd_unit_path() {
  printf '%s' "/etc/systemd/system/$(claw_pool_systemd_unit).service"
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
  [[ -f "$(claw_pool_systemd_unit_path)" ]]
}

claw_pool_systemd_active() {
  claw_pool_sudo systemctl is-active --quiet "$(claw_pool_systemd_unit)" 2>/dev/null
}

claw_pool_write_systemd_unit() {
  local rpc_dir="$1" run_sh="$2" repo_root="$3"
  claw_pool_sudo tee "$(claw_pool_systemd_unit_path)" >/dev/null <<EOF
[Unit]
Description=claw-pool-daemon (docker_pool worker pool)
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

claw_pool_systemd_install_and_restart() {
  local rpc_dir="$1" run_sh="$2" repo_root="$3"
  claw_pool_write_systemd_unit "${rpc_dir}" "${run_sh}" "${repo_root}"
  claw_pool_sudo systemctl daemon-reload
  claw_pool_sudo systemctl enable "$(claw_pool_systemd_unit)"
  claw_pool_sudo systemctl restart "$(claw_pool_systemd_unit)"
}

claw_pool_systemd_stop() {
  if claw_pool_systemd_installed; then
    claw_pool_sudo systemctl stop "$(claw_pool_systemd_unit)" 2>/dev/null || true
  fi
}

claw_pool_systemd_main_pid() {
  claw_pool_sudo systemctl show "$(claw_pool_systemd_unit)" -p MainPID --value 2>/dev/null || true
}

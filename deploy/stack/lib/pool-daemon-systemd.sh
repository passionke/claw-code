#!/usr/bin/env bash
# Linux production: claw-sandbox under systemd (root; docker mount). Author: kejiqing
set -euo pipefail

claw_pool_systemd_unit() {
  printf '%s' "claw-sandbox"
}

claw_pool_systemd_unit_path() {
  printf '%s' "/etc/systemd/system/$(claw_pool_systemd_unit).service"
}

claw_pool_use_systemd() {
  [[ "$(uname -s)" == "Linux" ]] || return 1
  case "${CLAW_POOL_DAEMON_USE_SYSTEMD:-}" in
    0 | false | no | off) return 1 ;;
    1 | true | yes | on) ;;
  esac
  # shellcheck disable=SC1091
  source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/env-profile.sh"
  [[ "$(claw_deploy_profile_name)" == "production" ]] || return 1
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

claw_pool_systemd_write_unit() {
  local rpc_dir="$1" run_sh="$2" repo_root="$3"
  local unit
  unit="$(claw_pool_systemd_unit)"
  claw_pool_sudo tee "$(claw_pool_systemd_unit_path)" >/dev/null <<EOF
[Unit]
Description=claw-sandbox (worker pool HTTP RPC)
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

claw_pool_systemd_stop_legacy_profile_units() {
  local u
  for u in claw-pool-daemon claw-pool-daemon-strict claw-pool-daemon-relaxed; do
    if claw_pool_sudo systemctl list-unit-files "${u}.service" >/dev/null 2>&1; then
      claw_pool_sudo systemctl stop "${u}" 2>/dev/null || true
      claw_pool_sudo systemctl disable "${u}" 2>/dev/null || true
    fi
  done
}

claw_pool_systemd_install_and_restart() {
  local rpc_dir="$1" run_sh="$2" repo_root="$3"
  local unit
  unit="$(claw_pool_systemd_unit)"
  claw_pool_systemd_stop_legacy_profile_units
  claw_pool_systemd_write_unit "${rpc_dir}" "${run_sh}" "${repo_root}"
  claw_pool_sudo systemctl daemon-reload
  claw_pool_sudo systemctl enable "${unit}"
  claw_pool_sudo systemctl restart "${unit}"
}

claw_pool_systemd_stop() {
  local unit
  unit="$(claw_pool_systemd_unit)"
  if claw_pool_systemd_installed; then
    claw_pool_sudo systemctl stop "${unit}" 2>/dev/null || true
  fi
}

claw_pool_systemd_stop_via_docker() {
  local unit rt image lib_dir
  unit="$(claw_pool_systemd_unit)"
  [[ -f "$(claw_pool_systemd_unit_path)" ]] || return 1
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
  claw_pool_sudo systemctl show "$(claw_pool_systemd_unit)" -p MainPID --value 2>/dev/null || true
}

#!/usr/bin/env bash
# macOS: run claw-pool-daemon under launchd (survives agent/terminal session teardown). Author: kejiqing
set -euo pipefail

claw_pool_launchd_label() {
  local profile="${1:-}"
  if [[ -n "${profile}" ]]; then
    printf '%s' "com.claw.pool-daemon.${profile}"
  else
    printf '%s' "com.claw.pool-daemon"
  fi
}

claw_pool_launchd_domain() {
  printf 'gui/%s' "$(id -u)"
}

claw_pool_launchd_plist_path() {
  local rpc_dir="$1"
  local profile="${2:-}"
  if [[ -n "${profile}" ]]; then
    printf '%s/com.claw.pool-daemon.%s.plist' "${rpc_dir}" "${profile}"
  else
    printf '%s/com.claw.pool-daemon.plist' "${rpc_dir}"
  fi
}

# Write LaunchAgent plist; pool-daemon-run.sh loads pool-daemon.env. kejiqing
claw_pool_write_launchd_plist() {
  local rpc_dir="$1" run_sh="$2" log="$3" profile="${4:-}"
  local plist path_val label
  plist="$(claw_pool_launchd_plist_path "${rpc_dir}" "${profile}")"
  label="$(claw_pool_launchd_label "${profile}")"
  # launchd does not inherit shell PATH; podman lives under Homebrew on macOS arm64. kejiqing
  path_val="${PATH:-/usr/bin:/bin:/usr/sbin:/sbin:/opt/homebrew/bin:/usr/local/bin}"
  cat >"${plist}" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>${label}</string>
  <key>ProgramArguments</key>
  <array>
    <string>${run_sh}</string>
    <string>${rpc_dir}</string>
  </array>
  <key>RunAtLoad</key>
  <true/>
  <key>KeepAlive</key>
  <dict>
    <key>SuccessfulExit</key>
    <false/>
  </dict>
  <key>StandardOutPath</key>
  <string>${log}</string>
  <key>StandardErrorPath</key>
  <string>${log}</string>
  <key>ProcessType</key>
  <string>Background</string>
  <key>EnvironmentVariables</key>
  <dict>
    <key>PATH</key>
    <string>${path_val}</string>
  </dict>
</dict>
</plist>
EOF
}

claw_pool_launchd_bootout() {
  local profile="${1:-}"
  local domain label
  domain="$(claw_pool_launchd_domain)"
  label="$(claw_pool_launchd_label "${profile}")"
  launchctl bootout "${domain}/${label}" 2>/dev/null || true
  if [[ -z "${profile}" ]]; then
    claw_pool_launchd_bootout strict
    claw_pool_launchd_bootout relaxed
  fi
}

claw_pool_launchd_bootstrap() {
  local rpc_dir="$1" run_sh="$2" log="$3" profile="${4:-}"
  local plist domain
  plist="$(claw_pool_launchd_plist_path "${rpc_dir}" "${profile}")"
  domain="$(claw_pool_launchd_domain)"
  claw_pool_write_launchd_plist "${rpc_dir}" "${run_sh}" "${log}" "${profile}"
  claw_pool_launchd_bootout "${profile}"
  sleep 0.5
  launchctl bootstrap "${domain}" "${plist}"
}

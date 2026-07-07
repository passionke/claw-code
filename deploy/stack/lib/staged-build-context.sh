#!/usr/bin/env bash
# Minimal build context when host workspace dirs block podman context upload (macOS drwx------). Author: kejiqing
set -euo pipefail

# Print temp dir path with same layout as repo root (only files referenced by prebuilt Containerfiles).
claw_stage_prebuilt_build_context() {
  local root_dir="${1:?root_dir}"
  local stage
  stage="$(mktemp -d "${TMPDIR:-/tmp}/claw-build-ctx.XXXXXX")"

  mkdir -p \
    "${stage}/deploy/stack/.linux-artifacts/release" \
    "${stage}/deploy/e2b" \
    "${stage}/web/gateway-async-playground" \
    "${stage}/web/gateway-admin"

  cp "${root_dir}/deploy/stack/debian-bookworm-ustc.sources" "${stage}/deploy/stack/"
  cp "${root_dir}/deploy/stack/openvscode-settings.json" "${stage}/deploy/stack/" 2>/dev/null || true
  cp "${root_dir}/deploy/stack/.linux-artifacts/release/claw" \
    "${root_dir}/deploy/stack/.linux-artifacts/release/http-gateway-rs" \
    "${stage}/deploy/stack/.linux-artifacts/release/"
  cp "${root_dir}"/deploy/stack/claw.claw-vscode-*.vsix "${stage}/deploy/stack/" 2>/dev/null || true
  cp "${root_dir}/deploy/e2b/e2b_exec.py" "${stage}/deploy/e2b/"
  cp "${root_dir}/web/gateway-async-playground/server.py" \
    "${root_dir}/web/gateway-async-playground/index.html" \
    "${stage}/web/gateway-async-playground/"
  if [[ -d "${root_dir}/web/gateway-admin" ]]; then
    cp -R "${root_dir}/web/gateway-admin/." "${stage}/web/gateway-admin/"
  fi

  printf '%s\n' "${stage}"
}

claw_needs_staged_build_context() {
  local root_dir="${1:?root_dir}"
  [[ "$(uname -s)" == Darwin ]] || return 1
  local ws="${root_dir}/deploy/stack/claw-workspace"
  [[ -e "${ws}" ]] || return 1
  [[ -r "${ws}" && -x "${ws}" ]] && return 1
  return 0
}

claw_resolve_build_context() {
  local root_dir="${1:?root_dir}"
  if claw_needs_staged_build_context "${root_dir}"; then
    echo "==> staged build context (unreadable deploy/stack/claw-workspace on macOS host)" >&2
    claw_stage_prebuilt_build_context "${root_dir}"
  else
    printf '%s\n' "${root_dir}"
  fi
}

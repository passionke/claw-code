#!/usr/bin/env bash
# Pull claw-sandbox release image and install host binary under deploy/stack/.linux-artifacts/.
# Used by gateway.sh up --release (no host cargo). Author: kejiqing
set -euo pipefail

_LIB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=compose-include.sh
source "${_LIB_DIR}/compose-include.sh"
# shellcheck source=pool-daemon-binary.sh
source "${_LIB_DIR}/pool-daemon-binary.sh"
# shellcheck source=release-images.sh
source "${_LIB_DIR}/release-images.sh"

claw_sandbox_release_stamp_path() {
  local podman_dir="${1:?}"
  printf '%s\n' "${podman_dir}/.linux-artifacts/release/.claw-sandbox-release-tag"
}

# Derive CLAW_SANDBOX_IMAGE from GATEWAY_IMAGE when sticky pin omits it (older pins).
claw_derive_sandbox_image_from_gateway() {
  local gw="${GATEWAY_IMAGE:-}"
  [[ "${gw}" == *"/claw-code:"* ]] || return 1
  printf '%s\n' "${gw/claw-code/claw-sandbox}"
}

claw_active_release_tag_for_sandbox() {
  if [[ -n "${CLAW_IMAGE_RELEASE_TAG:-}" ]]; then
    printf '%s\n' "${CLAW_IMAGE_RELEASE_TAG}"
    return 0
  fi
  local gw="${GATEWAY_IMAGE:-}"
  if [[ "${gw}" == *":"* ]]; then
    printf '%s\n' "${gw##*:}"
    return 0
  fi
  return 1
}

claw_install_claw_sandbox_from_release() {
  local podman_dir="${1:?}"
  local out
  local stamp
  local image
  local tag
  local rt
  local cid

  out="$(claw_default_pool_daemon_bin "${podman_dir}")"
  stamp="$(claw_sandbox_release_stamp_path "${podman_dir}")"
  mkdir -p "$(dirname "${out}")"

  image="${CLAW_SANDBOX_IMAGE:-}"
  if [[ -z "${image}" ]]; then
    image="$(claw_derive_sandbox_image_from_gateway)" || true
  fi
  if [[ -z "${image}" ]]; then
    echo "error: CLAW_SANDBOX_IMAGE unset and cannot derive from GATEWAY_IMAGE" >&2
    echo "hint: ./deploy/stack/gateway.sh up --release release-vX.Y.Z" >&2
    return 1
  fi

  tag="$(claw_active_release_tag_for_sandbox)" || tag="${image##*:}"

  if [[ -x "${out}" ]] && [[ -f "${stamp}" ]] && [[ "$(cat "${stamp}")" == "${tag}" ]] \
    && [[ "${CLAW_POOL_REBUILD_DAEMON:-0}" != "1" ]]; then
    echo "==> reusing host claw-sandbox from release ${tag}: ${out}" >&2
    return 0
  fi

  rt="$(claw_container_runtime_cli)" || return 1
  claw_release_pull_image_if_needed "${rt}" "${image}"

  echo "==> installing host claw-sandbox from ${image} → ${out}" >&2
  cid="$("${rt}" create "${image}")"
  trap '[[ -n "${cid:-}" ]] && "${rt}" rm -f "${cid}" 2>/dev/null || true' RETURN
  "${rt}" cp "${cid}:/usr/local/bin/claw-sandbox" "${out}"
  "${rt}" rm -f "${cid}"
  trap - RETURN
  chmod +x "${out}"
  printf '%s\n' "${tag}" >"${stamp}"
  echo "==> claw-sandbox installed (${tag})" >&2
}

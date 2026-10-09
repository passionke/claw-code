#!/usr/bin/env bash
# ONE path. Author: kejiqing. Do not invent forks.
# Resolve CLAW_IMAGE_PREFIX from env (Nora / ACR / Nexus).
# shellcheck shell=bash
set -euo pipefail

claw_pack_prefix() {
  if [[ -n "${CLAW_IMAGE_PREFIX:-}" ]]; then
    printf '%s' "${CLAW_IMAGE_PREFIX%/}"
    return 0
  fi
  # Back-compat compose only — not a second publish path.
  if [[ -n "${NEXUS_PUSH_REGISTRY:-}" ]]; then
    local ns="${NEXUS_NS:-passionke}"
    printf '%s/%s' "${NEXUS_PUSH_REGISTRY%/}" "${ns}"
    return 0
  fi
  echo "error: set CLAW_IMAGE_PREFIX (or NEXUS_PUSH_REGISTRY+NEXUS_NS)" >&2
  return 1
}

claw_pack_registry_login() {
  local prefix host user pass
  prefix="$(claw_pack_prefix)"
  host="${prefix%%/*}"
  user="${CLAW_REGISTRY_USER:-${NEXUS_USER:-}}"
  pass="${CLAW_REGISTRY_PASSWORD:-${NEXUS_PASSWORD:-}}"
  if [[ -z "$user" || -z "$pass" ]]; then
    echo "error: set CLAW_REGISTRY_USER/PASSWORD (or NEXUS_USER/PASSWORD)" >&2
    return 1
  fi
  echo "$pass" | docker login "$host" -u "$user" --password-stdin
}

claw_pack_skopeo_push() {
  local local_ref="$1"
  local remote_ref="$2"
  shift 2
  local root
  root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
  chmod +x "$root/deploy/stack/lib/ci-push-acr-skopeo.sh"
  "$root/deploy/stack/lib/ci-push-acr-skopeo.sh" "$local_ref" "$remote_ref" "$@"
}

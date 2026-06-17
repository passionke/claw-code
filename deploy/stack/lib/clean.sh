#!/usr/bin/env bash
# Remove local Rust/build artifacts under the repo (not podman images/volumes). Author: kejiqing
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
RUST_DIR="${ROOT_DIR}/rust"
LINUX_ARTIFACTS="${ROOT_DIR}/deploy/stack/.linux-artifacts"
CLAW_WORKSPACE="${ROOT_DIR}/deploy/stack/claw-workspace"

clean_usage() {
  cat <<EOF
Usage: gateway.sh clean [options]

Default (no extra flags):
  - rust/target/ via cargo clean (debug + release)
  - deploy/stack/.linux-artifacts/ (Darwin podman-run compile output)

Options:
  --debug-only          Only remove rust/target/debug (keeps release binaries; often frees most GB)
  --podman-compile-cache
                        Remove podman volumes claw-cargo-registry + claw-cargo-git (next build re-downloads crates)
  --sccache-volume      Remove podman volume claw-sccache (compiler cache; next build recompiles from scratch)
  --prune-claw-images   Remove unused local images matching claw-gateway / claw-code (podman image prune)
  --workspace           Also remove deploy/stack/claw-workspace/ (ds sessions; destructive)
  -h, --help            Show this help

Examples:
  ./deploy/stack/gateway.sh clean --debug-only
  ./deploy/stack/gateway.sh clean --debug-only
  podman system prune -f   # broader; not run by this script unless --prune-claw-images
EOF
}

claw_dir_size() {
  local p="$1"
  if [[ -e "${p}" ]]; then
    local out=""
    if out="$(du -sh "${p}" 2>/dev/null)"; then
      awk '{print $1}' <<<"${out}"
    else
      printf '%s' "n/a"
    fi
  else
    printf '%s' "0"
  fi
}

CLAW_CLEAN_WORKSPACE=0
CLAW_CLEAN_DEBUG_ONLY=0
CLAW_CLEAN_PODMAN_CACHE=0
CLAW_CLEAN_SCCACHE_VOLUME=0
CLAW_PRUNE_CLAW_IMAGES=0
while [[ $# -gt 0 ]]; do
  case "$1" in
    -h | --help)
      clean_usage
      exit 0
      ;;
    --debug-only)
      CLAW_CLEAN_DEBUG_ONLY=1
      shift
      ;;
    --podman-compile-cache)
      CLAW_CLEAN_PODMAN_CACHE=1
      shift
      ;;
    --sccache-volume)
      CLAW_CLEAN_SCCACHE_VOLUME=1
      shift
      ;;
    --prune-claw-images)
      CLAW_PRUNE_CLAW_IMAGES=1
      shift
      ;;
    --workspace)
      CLAW_CLEAN_WORKSPACE=1
      shift
      ;;
    --*)
      echo "unknown clean option: $1" >&2
      clean_usage >&2
      exit 2
      ;;
    *)
      echo "unexpected argument: $1" >&2
      clean_usage >&2
      exit 2
      ;;
  esac
done

echo "==> gateway clean (build artifacts only)"
before_target="$(claw_dir_size "${RUST_DIR}/target")"
before_linux="$(claw_dir_size "${LINUX_ARTIFACTS}")"
before_ws="$(claw_dir_size "${CLAW_WORKSPACE}")"

if [[ "${CLAW_CLEAN_DEBUG_ONLY}" == "1" ]]; then
  if [[ -d "${RUST_DIR}/target/debug" ]]; then
    rm -rf "${RUST_DIR}/target/debug"
    echo "    removed ${RUST_DIR}/target/debug (release kept under target/release)"
  else
    echo "    rust/target/debug: already absent"
  fi
elif [[ -d "${RUST_DIR}/target" ]]; then
  if command -v cargo >/dev/null 2>&1; then
    (cd "${RUST_DIR}" && cargo clean)
  else
    rm -rf "${RUST_DIR}/target"
    echo "    removed ${RUST_DIR}/target (no cargo in PATH)"
  fi
else
  echo "    rust/target: already absent"
fi

if [[ -d "${LINUX_ARTIFACTS}" ]]; then
  rm -rf "${LINUX_ARTIFACTS}"
  echo "    removed ${LINUX_ARTIFACTS}"
else
  echo "    .linux-artifacts: already absent"
fi

if [[ "${CLAW_CLEAN_WORKSPACE}" == "1" ]]; then
  if [[ -d "${CLAW_WORKSPACE}" ]]; then
    rm -rf "${CLAW_WORKSPACE}"
    echo "    removed ${CLAW_WORKSPACE}"
  else
    echo "    claw-workspace: already absent"
  fi
fi

if [[ "${CLAW_CLEAN_PODMAN_CACHE}" == "1" ]]; then
  rt="$(command -v podman 2>/dev/null || command -v docker 2>/dev/null || true)"
  if [[ -n "${rt}" ]]; then
    for vol in claw-cargo-registry claw-cargo-git; do
      if "${rt}" volume exists "${vol}" 2>/dev/null; then
        "${rt}" volume rm "${vol}" 2>/dev/null && echo "    removed volume ${vol}" \
          || echo "    volume ${vol}: in use or rm failed (stop workers first)" >&2
      else
        echo "    volume ${vol}: absent"
      fi
    done
  else
    echo "    --podman-compile-cache: no podman/docker in PATH" >&2
  fi
fi

if [[ "${CLAW_CLEAN_SCCACHE_VOLUME}" == "1" ]]; then
  rt="$(command -v podman 2>/dev/null || command -v docker 2>/dev/null || true)"
  if [[ -n "${rt}" ]]; then
    if "${rt}" volume exists claw-sccache 2>/dev/null; then
      "${rt}" volume rm claw-sccache 2>/dev/null && echo "    removed volume claw-sccache" \
        || echo "    volume claw-sccache: in use or rm failed" >&2
    else
      echo "    volume claw-sccache: absent"
    fi
  else
    echo "    --sccache-volume: no podman/docker in PATH" >&2
  fi
fi

if [[ "${CLAW_PRUNE_CLAW_IMAGES}" == "1" ]]; then
  rt="$(command -v podman 2>/dev/null || command -v docker 2>/dev/null || true)"
  if [[ -n "${rt}" ]]; then
    # Dangling + unused claw-tagged images only (does not remove running containers' images).
    "${rt}" image prune -f --filter "label=io.podman.compose.project=claw" 2>/dev/null || true
    while read -r img; do
      [[ -n "${img}" ]] || continue
      "${rt}" rmi -f "${img}" 2>/dev/null || true
    done < <("${rt}" images --format '{{.ID}} {{.Repository}}' 2>/dev/null \
      | awk '/claw-gateway|claw-code|claw-gateway-playground/ { print $1 }' | sort -u)
    echo "    pruned unused claw-* images (see: ${rt} images)"
  fi
fi

after_total="n/a"
if _repo_size="$(du -sh "${ROOT_DIR}" 2>/dev/null)"; then
  after_total="$(awk '{print $1}' <<<"${_repo_size}")"
fi
echo "==> freed (was): rust/target=${before_target} .linux-artifacts=${before_linux} workspace=${before_ws}"
echo "==> repo total now: ${after_total}"
if [[ "${CLAW_CLEAN_PODMAN_CACHE}" != "1" && "${CLAW_CLEAN_SCCACHE_VOLUME}" != "1" && "${CLAW_PRUNE_CLAW_IMAGES}" != "1" ]]; then
  echo "    tip: --debug-only (keep release) | --podman-compile-cache | --sccache-volume | --prune-claw-images"
fi

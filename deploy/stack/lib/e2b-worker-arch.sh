#!/usr/bin/env bash
# Resolve linux arch for e2b worker template + claw compile (self-hosted fleet = amd64). Author: kejiqing

# Prints: arm64 | amd64
claw_e2b_worker_linux_arch() {
  local raw="${CLAW_E2B_WORKER_ARCH:-}"
  if [[ -n "${raw}" ]]; then
    case "${raw}" in
      arm64 | aarch64) printf '%s\n' arm64; return 0 ;;
      amd64 | x86_64) printf '%s\n' amd64; return 0 ;;
      *)
        echo "e2b worker arch: unsupported CLAW_E2B_WORKER_ARCH=${raw}" >&2
        return 1
        ;;
    esac
  fi
  if [[ -n "${CLAW_LINUX_COMPILE_PLATFORM:-}" || -n "${CLAW_E2B_TEMPLATE_PLATFORM:-}" ]]; then
    # shellcheck source=/dev/null
    source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/linux-compile.sh"
    claw_linux_compile_arch
    return $?
  fi
  # Self-hosted e2b (10.8.0.x) and FC worker nodes are linux/amd64. Mac dev cross-compiles.
  printf '%s\n' amd64
}

claw_e2b_worker_linux_platform() {
  printf 'linux/%s\n' "$(claw_e2b_worker_linux_arch)"
}

claw_e2b_ttyd_asset_name() {
  local arch
  arch="$(claw_e2b_worker_linux_arch)"
  case "${arch}" in
    arm64) printf '%s\n' aarch64 ;;
    amd64) printf '%s\n' x86_64 ;;
    *)
      echo "e2b ttyd: unsupported arch ${arch}" >&2
      return 1
      ;;
  esac
}

# file(1) probe must match ELF for the target arch.
claw_e2b_elf_arch_ok() {
  local probe="$1"
  local arch="$2"
  [[ "${probe}" == *"ELF"* ]] || return 1
  case "${arch}" in
    arm64) [[ "${probe}" == *"ARM aarch64"* || "${probe}" == *"aarch64"* ]] ;;
    amd64) [[ "${probe}" == *"x86-64"* || "${probe}" == *"x86_64"* ]] ;;
    *) return 1 ;;
  esac
}

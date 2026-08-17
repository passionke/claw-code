#!/usr/bin/env bash
# Ensure claw-rust-compile image (mold + sccache) exists for linux-compile. Author: kejiqing

claw_rust_compile_platform_suffix() {
  local raw="${CLAW_LINUX_COMPILE_PLATFORM:-}"
  if [[ -z "${raw}" ]]; then
    return 0
  fi
  case "${raw}" in
    linux/amd64 | amd64 | x86_64) printf '%s\n' amd64 ;;
    linux/arm64 | arm64 | aarch64) printf '%s\n' arm64 ;;
    *)
      echo "rust compile image: unsupported CLAW_LINUX_COMPILE_PLATFORM=${raw}" >&2
      return 1
      ;;
  esac
}

claw_rust_compile_image_name() {
  local tag="${CLAW_RUST_IMAGE_TAG:-1.88-bookworm}"
  local suffix
  suffix="$(claw_rust_compile_platform_suffix 2>/dev/null || true)"
  if [[ -n "${suffix}" ]]; then
    printf 'claw-rust-compile:%s-%s\n' "${tag}" "${suffix}"
  else
    printf 'claw-rust-compile:%s\n' "${tag}"
  fi
}

claw_rust_compile_apt_mirror() {
  if [[ -n "${CLAW_USE_CN_APT_MIRROR:-}" ]]; then
    printf '%s\n' "${CLAW_USE_CN_APT_MIRROR}"
    return 0
  fi
  if [[ "${GITHUB_ACTIONS:-}" == "true" ]]; then
    printf '0\n'
  else
    printf '1\n'
  fi
}

# docker has no `image exists` (podman does); inspect is the portable check. Author: kejiqing
claw_rust_compile_image_present() {
  local container_cli="$1"
  local image_name="$2"
  "${container_cli}" image inspect "${image_name}" >/dev/null 2>&1
}

claw_ensure_rust_compile_image() {
  local root_dir="$1"
  local container_cli="$2"
  local reg="$3"
  local image_name
  local apt_cn
  local -a platform_args=()
  image_name="$(claw_rust_compile_image_name)"
  apt_cn="$(claw_rust_compile_apt_mirror)"
  if [[ -n "${CLAW_LINUX_COMPILE_PLATFORM:-}" ]]; then
    platform_args=(--platform "${CLAW_LINUX_COMPILE_PLATFORM}")
  fi
  if claw_rust_compile_image_present "${container_cli}" "${image_name}"; then
    echo "==> reuse compile image ${image_name}" >&2
    printf '%s\n' "${image_name}"
    return 0
  fi
  # shellcheck source=/dev/null
  source "${root_dir}/deploy/stack/rust-version.env"
  local rust_base="${reg}/library/rust:${CLAW_RUST_IMAGE_TAG}"
  echo "==> building compile image ${image_name} (FROM ${rust_base}${CLAW_LINUX_COMPILE_PLATFORM:+, platform=${CLAW_LINUX_COMPILE_PLATFORM}}; apt_cn=${apt_cn})" >&2
  # Empty array + `set -u` breaks `"${platform_args[@]}"` on bash 3.2/macOS. Author: kejiqing
  if [[ ${#platform_args[@]} -gt 0 ]]; then
    "${container_cli}" build \
      "${platform_args[@]}" \
      --build-arg "RUST_BASE_IMAGE=${rust_base}" \
      --build-arg "CLAW_USE_CN_APT_MIRROR=${apt_cn}" \
      -f "${root_dir}/deploy/stack/Containerfile.rust-compile" \
      -t "${image_name}" \
      "${root_dir}" >&2
  else
    "${container_cli}" build \
      --build-arg "RUST_BASE_IMAGE=${rust_base}" \
      --build-arg "CLAW_USE_CN_APT_MIRROR=${apt_cn}" \
      -f "${root_dir}/deploy/stack/Containerfile.rust-compile" \
      -t "${image_name}" \
      "${root_dir}" >&2
  fi
  printf '%s\n' "${image_name}"
}

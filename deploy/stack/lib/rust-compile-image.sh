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

# GHCR ref for CI prebuilt compile image (linux-compile-once). Author: kejiqing
claw_rust_compile_ghcr_ref() {
  if [[ -n "${CLAW_RUST_COMPILE_IMAGE:-}" ]]; then
    printf '%s\n' "${CLAW_RUST_COMPILE_IMAGE}"
    return 0
  fi
  if [[ "${CLAW_LINUX_COMPILE_CI:-0}" == "1" ]] && [[ -n "${GITHUB_REPOSITORY_OWNER:-}" ]]; then
    local tag="${CLAW_RUST_IMAGE_TAG:-1.88-bookworm}"
    local suffix
    suffix="$(claw_rust_compile_platform_suffix 2>/dev/null || true)"
    if [[ -n "${suffix}" ]]; then
      printf 'ghcr.io/%s/claw-rust-compile:%s-%s\n' "${GITHUB_REPOSITORY_OWNER}" "${tag}" "${suffix}"
    else
      printf 'ghcr.io/%s/claw-rust-compile:%s\n' "${GITHUB_REPOSITORY_OWNER}" "${tag}"
    fi
    return 0
  fi
  return 1
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

claw_rust_compile_try_pull_ghcr() {
  local container_cli="$1"
  local ghcr_ref="$2"
  local local_name="$3"
  local -a platform_args=()

  if [[ -n "${CLAW_LINUX_COMPILE_PLATFORM:-}" ]]; then
    platform_args=(--platform "${CLAW_LINUX_COMPILE_PLATFORM}")
  fi

  echo "==> pull compile image ${ghcr_ref}" >&2
  if [[ ${#platform_args[@]} -gt 0 ]]; then
    if ! "${container_cli}" pull "${platform_args[@]}" "${ghcr_ref}" >&2; then
      return 1
    fi
  elif ! "${container_cli}" pull "${ghcr_ref}" >&2; then
    return 1
  fi
  "${container_cli}" tag "${ghcr_ref}" "${local_name}" >&2
  echo "==> tagged ${ghcr_ref} → ${local_name}" >&2
  return 0
}

claw_rust_compile_build_local() {
  local root_dir="$1"
  local container_cli="$2"
  local reg="$3"
  local image_name="$4"
  local apt_cn="$5"
  local -a platform_args=()

  if [[ -n "${CLAW_LINUX_COMPILE_PLATFORM:-}" ]]; then
    platform_args=(--platform "${CLAW_LINUX_COMPILE_PLATFORM}")
  fi
  # shellcheck source=/dev/null
  source "${root_dir}/deploy/stack/rust-version.env"
  local rust_base="${reg}/library/rust:${CLAW_RUST_IMAGE_TAG}"
  echo "==> building compile image ${image_name} (FROM ${rust_base}${CLAW_LINUX_COMPILE_PLATFORM:+, platform=${CLAW_LINUX_COMPILE_PLATFORM}}; apt_cn=${apt_cn})" >&2
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
}

claw_ensure_rust_compile_image() {
  local root_dir="$1"
  local container_cli="$2"
  local reg="$3"
  local image_name
  local apt_cn
  local ghcr_ref=""

  image_name="$(claw_rust_compile_image_name)"
  apt_cn="$(claw_rust_compile_apt_mirror)"

  if claw_rust_compile_image_present "${container_cli}" "${image_name}"; then
    echo "==> reuse compile image ${image_name}" >&2
    printf '%s\n' "${image_name}"
    return 0
  fi

  if ghcr_ref="$(claw_rust_compile_ghcr_ref)"; then
    if claw_rust_compile_try_pull_ghcr "${container_cli}" "${ghcr_ref}" "${image_name}"; then
      printf '%s\n' "${image_name}"
      return 0
    fi
    echo "==> GHCR pull failed for ${ghcr_ref}; building locally" >&2
  fi

  claw_rust_compile_build_local "${root_dir}" "${container_cli}" "${reg}" "${image_name}" "${apt_cn}"
  printf '%s\n' "${image_name}"
}

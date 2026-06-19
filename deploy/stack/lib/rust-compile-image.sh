#!/usr/bin/env bash
# Ensure claw-rust-compile image (mold + sccache) exists for linux-compile. Author: kejiqing

claw_rust_compile_image_name() {
  local tag="${CLAW_RUST_IMAGE_TAG:-1.88-bookworm}"
  printf 'claw-rust-compile:%s\n' "${tag}"
}

claw_ensure_rust_compile_image() {
  local root_dir="$1"
  local container_cli="$2"
  local reg="$3"
  local image_name
  image_name="$(claw_rust_compile_image_name)"
  if "${container_cli}" image exists "${image_name}" 2>/dev/null; then
    printf '%s\n' "${image_name}"
    return 0
  fi
  # shellcheck source=/dev/null
  source "${root_dir}/deploy/stack/rust-version.env"
  local rust_base="${reg}/library/rust:${CLAW_RUST_IMAGE_TAG}"
  echo "==> building compile image ${image_name} (FROM ${rust_base})" >&2
  # shellcheck disable=SC2086
  "${container_cli}" build \
    --build-arg "RUST_BASE_IMAGE=${rust_base}" \
    -f "${root_dir}/deploy/stack/Containerfile.rust-compile" \
    -t "${image_name}" \
    "${root_dir}" >&2
  printf '%s\n' "${image_name}"
}

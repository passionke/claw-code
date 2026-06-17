#!/usr/bin/env bash
# Compile Linux release binaries via container run (Darwin + CI). Artifacts land in
# deploy/stack/.linux-artifacts/release/ — image build only COPY, no cargo in `podman build`.
# Author: kejiqing
set -euo pipefail

claw_linux_compile_release() {
  local root_dir="$1"
  local container_cli="$2"
  local rust_image="$3"
  local use_cn_cargo="$4"

  local rust_dir="${root_dir}/rust"
  local sandbox_dir="${root_dir}/sandbox"
  local out_root="${root_dir}/deploy/stack/.linux-artifacts"
  local out_dir="${out_root}/release"
  mkdir -p "${out_dir}"

  mkdir -p "${rust_dir}/.cargo"
  if [[ ! -f "${rust_dir}/.cargo/config.toml" ]]; then
    cp "${rust_dir}/.cargo/config.toml.example" "${rust_dir}/.cargo/config.toml"
  elif [[ "${use_cn_cargo}" == "1" ]] && ! grep -q 'rsproxy-sparse' "${rust_dir}/.cargo/config.toml" 2>/dev/null; then
    cp "${rust_dir}/.cargo/config.toml.example" "${rust_dir}/.cargo/config.toml"
  fi

  mkdir -p "${sandbox_dir}/.cargo"
  if [[ ! -f "${sandbox_dir}/.cargo/config.toml" ]]; then
    cp "${sandbox_dir}/.cargo/config.toml.example" "${sandbox_dir}/.cargo/config.toml"
  fi

  echo "linux compile: ${container_cli} run (registry/git/target/sccache volumes persist across runs)"
  echo "  source: ${rust_dir}"
  echo "  target: ${out_dir}"
  echo "  image: ${rust_image}"

  local linux_arch
  linux_arch="$(uname -m)"
  case "${linux_arch}" in
    arm64 | aarch64) linux_arch=arm64 ;;
    x86_64 | amd64) linux_arch=amd64 ;;
    *)
      echo "linux compile: unsupported host arch ${linux_arch}" >&2
      exit 1
      ;;
  esac
  echo "  platform: linux/${linux_arch}"

  # shellcheck disable=SC2086
  # shellcheck source=/dev/null
  source "${root_dir}/deploy/stack/rust-version.env"
  export CLAW_RUST_VERSION

  local rustup_dist rustup_root
  rustup_dist="https://static.rust-lang.org"
  rustup_root="https://static.rust-lang.org/rustup"
  if [[ "${use_cn_cargo}" == "1" ]]; then
    rustup_dist="https://mirrors.ustc.edu.cn/rust-static"
    rustup_root="https://mirrors.ustc.edu.cn/rust-static/rustup"
  fi

  local sccache_size="${CLAW_SCCACHE_CACHE_SIZE:-10G}"

  local -a vol_args=()
  if [[ "${CLAW_LINUX_COMPILE_CI:-0}" == "1" ]]; then
    local ci_cache="${root_dir}/.ci-cache"
    mkdir -p "${ci_cache}/cargo-registry" "${ci_cache}/cargo-git" "${ci_cache}/sccache"
    vol_args=(
      -v "${ci_cache}/cargo-registry:/usr/local/cargo/registry:Z"
      -v "${ci_cache}/cargo-git:/usr/local/cargo/git:Z"
      -v "${ci_cache}/sccache:/root/.cache/sccache:Z"
    )
    echo "  ci cache: ${ci_cache}"
  else
    vol_args=(
      -v claw-cargo-registry:/usr/local/cargo/registry
      -v claw-cargo-git:/usr/local/cargo/git
      -v claw-sccache:/root/.cache/sccache
    )
  fi

  # shellcheck disable=SC2086
  "${container_cli}" run --rm --platform "linux/${linux_arch}" \
    -e "CLAW_RUST_VERSION=${CLAW_RUST_VERSION}" \
    -e "RUSTUP_DIST_SERVER=${rustup_dist}" \
    -e "RUSTUP_UPDATE_ROOT=${rustup_root}" \
    -e "RUSTC_WRAPPER=sccache" \
    -e "SCCACHE_DIR=/root/.cache/sccache" \
    -e "SCCACHE_CACHE_SIZE=${sccache_size}" \
    -v "${root_dir}:/workspace:Z" \
    "${vol_args[@]}" \
    -v "${out_root}:/artifacts:Z" \
    -w /workspace/rust \
    "${rust_image}" \
    bash -c '
      set -eu
      export PATH=/usr/local/cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin
      export CARGO_TARGET_DIR=/artifacts
      if [ -f .cargo/config.toml.example ] && [ ! -f .cargo/config.toml ]; then
        cp .cargo/config.toml.example .cargo/config.toml
      fi
      got=$(rustc --version | awk "{print \$2}")
      want="${CLAW_RUST_VERSION:?CLAW_RUST_VERSION unset}"
      if [ "$got" != "$want" ]; then
        echo "rustc version mismatch: want $want got $got" >&2
        exit 1
      fi
      echo "rustc $got (locked)"
      if command -v sccache >/dev/null 2>&1; then
        sccache --show-stats || true
      fi
      cargo build --release -p rusty-claude-cli --bin claw \
        -p http-gateway-rs --bin http-gateway-rs
      cd /workspace/sandbox
      export CARGO_TARGET_DIR=/artifacts
      if [ -f .cargo/config.toml.example ] && [ ! -f .cargo/config.toml ]; then
        cp .cargo/config.toml.example .cargo/config.toml
      fi
      cargo build --release -p claw-sandbox-server
      if command -v sccache >/dev/null 2>&1; then
        sccache --show-stats || true
      fi
      ls -la /artifacts/release/http-gateway-rs /artifacts/release/claw /artifacts/release/claw-sandbox
    '

  for bin in http-gateway-rs claw claw-sandbox; do
    if [[ ! -f "${out_dir}/${bin}" ]]; then
      echo "error: missing ${out_dir}/${bin} after linux compile" >&2
      exit 1
    fi
  done
  echo "linux compile: ok → ${out_dir}"
}

if [[ "${BASH_SOURCE[0]}" == "${0}" ]]; then
  ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
  # shellcheck source=/dev/null
  source "${ROOT_DIR}/deploy/stack/lib/compose-include.sh"
  CONTAINER_CLI="$(claw_container_runtime_cli)" || exit 1
  if [[ "${CLAW_USE_DOCKER_IO:-}" == "1" ]] || [[ "${GITHUB_ACTIONS:-}" == "true" ]]; then
    REG="docker.io"
  else
    REG="${CONTAINER_BASE_REGISTRY:-docker.1ms.run}"
    REG="${REG%/}"
  fi
  CN_FLAG=0
  if [[ "${GITHUB_ACTIONS:-}" != "true" ]] && [[ "${CLAW_USE_CN_CRATES_MIRROR:-0}" == "1" || "${CLAW_USE_CN_RUST_MIRROR:-0}" == "1" ]]; then
    CN_FLAG=1
  fi
  # shellcheck source=/dev/null
  source "${ROOT_DIR}/deploy/stack/lib/rust-compile-image.sh"
  COMPILE_IMAGE="$(claw_ensure_rust_compile_image "${ROOT_DIR}" "${CONTAINER_CLI}" "${REG}")"
  claw_linux_compile_release "${ROOT_DIR}" "${CONTAINER_CLI}" "${COMPILE_IMAGE}" "${CN_FLAG}"
fi

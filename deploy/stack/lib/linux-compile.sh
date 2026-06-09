#!/usr/bin/env bash
# Compile Linux release binaries via `podman run` (Darwin local path). Artifacts land in
# deploy/stack/.linux-artifacts/release/ — image build only COPY, no crates.io in `podman build`.
# Author: kejiqing
set -euo pipefail

claw_linux_compile_release() {
  local root_dir="$1"
  local container_cli="$2"
  local rust_image="$3"
  local use_cn_cargo="$4"

  local rust_dir="${root_dir}/rust"
  local out_root="${root_dir}/deploy/stack/.linux-artifacts"
  local out_dir="${out_root}/release"
  mkdir -p "${out_dir}"

  mkdir -p "${rust_dir}/.cargo"
  if [[ "${use_cn_cargo}" == "1" ]] && [[ ! -f "${rust_dir}/.cargo/config.toml" ]]; then
    cp "${rust_dir}/.cargo/config.toml.example" "${rust_dir}/.cargo/config.toml"
  fi

  echo "linux compile: ${container_cli} run (registry/git/target volumes persist across runs)"
  echo "  source: ${rust_dir}"
  echo "  target: ${out_dir}"

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

  # shellcheck disable=SC2086
  "${container_cli}" run --rm --platform "linux/${linux_arch}" \
    -e "CLAW_RUST_VERSION=${CLAW_RUST_VERSION}" \
    -e "RUSTUP_DIST_SERVER=${rustup_dist}" \
    -e "RUSTUP_UPDATE_ROOT=${rustup_root}" \
    -v "${rust_dir}:/build:Z" \
    -v claw-cargo-registry:/usr/local/cargo/registry \
    -v claw-cargo-git:/usr/local/cargo/git \
    -v "${out_root}:/artifacts:Z" \
    -w /build \
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
      # Darwin local: pool-daemon runs as host Mach-O binary, not from gateway image. kejiqing
      cargo build --release -p rusty-claude-cli --bin claw \
        -p http-gateway-rs --bin http-gateway-rs
      ls -la /artifacts/release/http-gateway-rs /artifacts/release/claw
    '

  for bin in http-gateway-rs claw; do
    if [[ ! -f "${out_dir}/${bin}" ]]; then
      echo "error: missing ${out_dir}/${bin} after linux compile" >&2
      exit 1
    fi
  done
  echo "linux compile: ok → ${out_dir}"
}

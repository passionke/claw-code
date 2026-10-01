#!/usr/bin/env bash
# Compile Linux release binaries via container run (Darwin + CI). Artifacts land in
# deploy/stack/.linux-artifacts/release/ — image build only COPY, no cargo in `podman build`.
# Author: kejiqing
set -euo pipefail

# Resolve container platform arch (override via CLAW_LINUX_COMPILE_PLATFORM=linux/amd64). Author: kejiqing
claw_linux_compile_arch() {
  local raw="${CLAW_LINUX_COMPILE_PLATFORM:-}"
  if [[ -n "${raw}" ]]; then
    case "${raw}" in
      linux/amd64 | amd64 | x86_64) printf '%s\n' amd64; return 0 ;;
      linux/arm64 | arm64 | aarch64) printf '%s\n' arm64; return 0 ;;
      *)
        echo "linux compile: unsupported CLAW_LINUX_COMPILE_PLATFORM=${raw}" >&2
        return 1
        ;;
    esac
  fi
  case "$(uname -m)" in
    arm64 | aarch64) printf '%s\n' arm64 ;;
    x86_64 | amd64) printf '%s\n' amd64 ;;
    *)
      echo "linux compile: unsupported host arch $(uname -m)" >&2
      return 1
      ;;
  esac
}

# CI: drop cargo target debris; keep only release binaries for artifact upload. Author: kejiqing
claw_linux_compile_prune_ci_bins() {
  local out_dir="$1"
  local item base keep
  shopt -s nullglob
  for item in "${out_dir}"/*; do
    base="$(basename "${item}")"
    keep=0
    case "${base}" in
      claw | http-gateway-rs) keep=1 ;;
    esac
    if [[ "${keep}" -eq 0 ]]; then
      rm -rf "${item}"
    fi
  done
  shopt -u nullglob
}

claw_linux_compile_release() {
  local root_dir="$1"
  local container_cli="$2"
  local rust_image="$3"
  local use_cn_cargo="$4"

  local rust_dir="${root_dir}/rust"
  local out_root="${root_dir}/deploy/stack/.linux-artifacts"
  local out_dir="${out_root}/release"
  if [[ "${CLAW_LINUX_COMPILE_CI:-0}" == "1" ]]; then
    rm -rf "${out_root}" 2>/dev/null || true
    if [[ -d "${out_root}" ]] && command -v docker >/dev/null 2>&1; then
      docker run --rm -v "${root_dir}:/w:rw" alpine:3.20 rm -rf /w/deploy/stack/.linux-artifacts 2>/dev/null || true
    fi
  fi
  mkdir -p "${out_dir}"

  mkdir -p "${rust_dir}/.cargo"
  if [[ ! -f "${rust_dir}/.cargo/config.toml" ]]; then
    cp "${rust_dir}/.cargo/config.toml.example" "${rust_dir}/.cargo/config.toml"
  elif [[ "${use_cn_cargo}" == "1" ]] && ! grep -q 'rsproxy-sparse' "${rust_dir}/.cargo/config.toml" 2>/dev/null; then
    cp "${rust_dir}/.cargo/config.toml.example" "${rust_dir}/.cargo/config.toml"
  fi

  echo "linux compile: ${container_cli} run (registry/git/target/sccache volumes persist across runs)"
  echo "  source: ${rust_dir}"
  echo "  target: ${out_dir}"
  echo "  image: ${rust_image}"

  local linux_arch
  linux_arch="$(claw_linux_compile_arch)"
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

  local ci_cache=""
  local -a vol_args=()
  if [[ "${CLAW_LINUX_COMPILE_CI:-0}" == "1" ]]; then
    ci_cache="${root_dir}/.ci-cache"
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

  local -a uid_args=()
  uid_args=(-e "CLAW_HOST_UID=$(id -u)" -e "CLAW_HOST_GID=$(id -g)")

  # Only forward when set — empty CARGO_BUILD_JOBS makes cargo error. Author: kejiqing
  local -a cargo_env_args=()
  [[ -n "${CARGO_BUILD_JOBS:-}" ]] && cargo_env_args+=(-e "CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS}")
  [[ -n "${CARGO_INCREMENTAL:-}" ]] && cargo_env_args+=(-e "CARGO_INCREMENTAL=${CARGO_INCREMENTAL}")
  [[ -n "${CARGO_PROFILE_RELEASE_CODEGEN_UNITS:-}" ]] && \
    cargo_env_args+=(-e "CARGO_PROFILE_RELEASE_CODEGEN_UNITS=${CARGO_PROFILE_RELEASE_CODEGEN_UNITS}")

  # utoipa-swagger-ui build.rs curls a zip at compile time (not our shell). Prefetch on the
  # host into .ci-cache (survives actions/cache) and pass file:// so the container never
  # hits the network for this. Author: kejiqing
  local swagger_ver="v5.17.14"
  local swagger_zip_name="swagger-ui-${swagger_ver}.zip"
  local swagger_src="${SWAGGER_UI_DOWNLOAD_URL:-}"
  local swagger_dir swagger_zip attempt
  if [[ -z "${swagger_src}" ]]; then
    if [[ "${use_cn_cargo}" == "1" \
      || "${CLAW_USE_CN_CRATES_MIRROR:-0}" == "1" \
      || "${CLAW_USE_CN_APT_MIRROR:-0}" == "1" ]]; then
      # ghfast: 21 probed 3×200 ~5s; raw github.com via TUN still TLS-EOF. Author: kejiqing
      swagger_src="https://ghfast.top/https://github.com/swagger-api/swagger-ui/archive/refs/tags/${swagger_ver}.zip"
    else
      swagger_src="https://github.com/swagger-api/swagger-ui/archive/refs/tags/${swagger_ver}.zip"
    fi
  fi
  if [[ "${CLAW_LINUX_COMPILE_CI:-0}" == "1" && -n "${ci_cache}" ]]; then
    swagger_dir="${ci_cache}/swagger-ui"
  else
    swagger_dir="${out_root}/.swagger-ui-cache"
  fi
  mkdir -p "${swagger_dir}"
  swagger_zip="${swagger_dir}/${swagger_zip_name}"
  if [[ "${swagger_src}" == file://* ]]; then
    cargo_env_args+=(-e "SWAGGER_UI_DOWNLOAD_URL=${swagger_src}")
  else
    if [[ ! -s "${swagger_zip}" ]]; then
      echo "linux compile: fetch swagger-ui ${swagger_ver} → ${swagger_zip}"
      echo "  url: ${swagger_src}"
      for attempt in 1 2 3 4 5; do
        if curl --http1.1 -fL --connect-timeout 20 --max-time 180 \
          --retry 2 --retry-delay 2 --retry-all-errors \
          -o "${swagger_zip}.partial" "${swagger_src}"; then
          mv -f "${swagger_zip}.partial" "${swagger_zip}"
          break
        fi
        echo "linux compile: swagger download attempt ${attempt}/5 failed" >&2
        rm -f "${swagger_zip}.partial"
        sleep $((attempt * 2))
      done
      if [[ ! -s "${swagger_zip}" ]]; then
        echo "error: swagger-ui download failed after retries: ${swagger_src}" >&2
        exit 1
      fi
    else
      echo "linux compile: reuse cached swagger-ui ${swagger_zip}"
    fi
    cargo_env_args+=(-e "SWAGGER_UI_DOWNLOAD_URL=file:///swagger-ui/${swagger_zip_name}")
    vol_args+=(-v "${swagger_dir}:/swagger-ui:ro")
  fi

  # shellcheck disable=SC2086
  local compile_rc=0
  "${container_cli}" run --rm --pull=never --platform "linux/${linux_arch}" \
    -e "CLAW_RUST_VERSION=${CLAW_RUST_VERSION}" \
    -e "RUSTUP_DIST_SERVER=${rustup_dist}" \
    -e "RUSTUP_UPDATE_ROOT=${rustup_root}" \
    -e "RUSTC_WRAPPER=sccache" \
    -e "SCCACHE_DIR=/root/.cache/sccache" \
    -e "SCCACHE_CACHE_SIZE=${sccache_size}" \
    -e "RUST_MIN_STACK=${RUST_MIN_STACK:-16777216}" \
    "${cargo_env_args[@]+"${cargo_env_args[@]}"}" \
    "${uid_args[@]+"${uid_args[@]}"}" \
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
      if command -v sccache >/dev/null 2>&1; then
        sccache --show-stats || true
      fi
      ls -la /artifacts/release/http-gateway-rs /artifacts/release/claw
      find /artifacts/release -mindepth 1 -maxdepth 1 \
        ! -name claw ! -name http-gateway-rs -exec rm -rf {} + 2>/dev/null || true
      if [ -n "${CLAW_HOST_UID:-}" ] && [ -n "${CLAW_HOST_GID:-}" ]; then
        chown -R "${CLAW_HOST_UID}:${CLAW_HOST_GID}" /artifacts
        chown -R "${CLAW_HOST_UID}:${CLAW_HOST_GID}" \
          /usr/local/cargo/registry /usr/local/cargo/git /root/.cache/sccache
      fi
    ' || compile_rc=$?

  # cargo failure skips in-container chown — always reclaim .ci-cache for checkout/cache. Author: kejiqing
  if [[ "${CLAW_LINUX_COMPILE_CI:-0}" == "1" && -n "${ci_cache}" && -d "${ci_cache}" ]]; then
    local host_uid host_gid
    host_uid="$(id -u)"
    host_gid="$(id -g)"
    if ! chown -R "${host_uid}:${host_gid}" "${ci_cache}" 2>/dev/null; then
      docker run --rm -v "${root_dir}:/w:rw" alpine:3.20 \
        chown -R "${host_uid}:${host_gid}" /w/.ci-cache || true
    fi
    echo "linux compile: reclaim ci-cache ownership → ${host_uid}:${host_gid} (compile_rc=${compile_rc})"
  fi
  if [[ "${compile_rc}" -ne 0 ]]; then
    return "${compile_rc}"
  fi

  if [[ "${CLAW_LINUX_COMPILE_CI:-0}" == "1" ]]; then
    claw_linux_compile_prune_ci_bins "${out_dir}"
    # CI uses CARGO_TARGET_DIR=.linux-artifacts — intermediate deps/build can be tens of GB.
    # Keep only the two release bins; wipe the rest of the target tree immediately. Author: kejiqing
    local bin_tmp host_uid host_gid
    bin_tmp="$(mktemp -d "${TMPDIR:-/tmp}/claw-linux-bins.XXXXXX")"
    host_uid="$(id -u)"
    host_gid="$(id -g)"
    for bin in http-gateway-rs claw; do
      if [[ ! -f "${out_dir}/${bin}" ]]; then
        echo "error: missing ${out_dir}/${bin} after linux compile" >&2
        rm -rf "${bin_tmp}"
        exit 1
      fi
      cp -a "${out_dir}/${bin}" "${bin_tmp}/"
    done
    echo "linux compile: drop CARGO_TARGET_DIR debris under ${out_root} (keep bins only)"
    rm -rf "${out_root}" 2>/dev/null || true
    if [[ -d "${out_root}" ]] && command -v docker >/dev/null 2>&1; then
      docker run --rm -v "${root_dir}:/w:rw" alpine:3.20 rm -rf /w/deploy/stack/.linux-artifacts
    fi
    mkdir -p "${out_dir}"
    cp -a "${bin_tmp}/." "${out_dir}/"
    rm -rf "${bin_tmp}"
    # Accidental host rust/target (if someone built outside CARGO_TARGET_DIR). Author: kejiqing
    if [[ -d "${rust_dir}/target" ]]; then
      echo "linux compile: remove stray ${rust_dir}/target"
      rm -rf "${rust_dir}/target" 2>/dev/null \
        || docker run --rm -v "${root_dir}:/w:rw" alpine:3.20 rm -rf /w/rust/target \
        || true
    fi
    # Docker writes registry/sccache as root; actions/cache must tar as runner user. Author: kejiqing
    if [[ -n "${ci_cache}" ]] && [[ -d "${ci_cache}" ]]; then
      if chown -R "${host_uid}:${host_gid}" "${ci_cache}" 2>/dev/null; then
        echo "linux compile: chown ci cache → ${host_uid}:${host_gid}"
      elif command -v docker >/dev/null 2>&1; then
        docker run --rm -v "${root_dir}:/w:rw" alpine:3.20 \
          chown -R "${host_uid}:${host_gid}" /w/.ci-cache
        echo "linux compile: chown ci cache via docker → ${host_uid}:${host_gid}"
      else
        echo "linux compile: warning: could not chown ${ci_cache} (actions/cache save may fail)" >&2
      fi
    fi
  fi
  for bin in http-gateway-rs claw; do
    if [[ ! -f "${out_dir}/${bin}" ]]; then
      echo "error: missing ${out_dir}/${bin} after linux compile" >&2
      exit 1
    fi
    if ! chmod +x "${out_dir}/${bin}" 2>/dev/null; then
      local host_uid host_gid
      host_uid="$(id -u)"
      host_gid="$(id -g)"
      if command -v docker >/dev/null 2>&1; then
        docker run --rm -v "${out_root}:/w:rw" alpine:3.20 \
          chown -R "${host_uid}:${host_gid}" /w
        chmod +x "${out_dir}/${bin}"
      else
        echo "error: cannot chmod ${out_dir}/${bin} (root-owned artifact?)" >&2
        exit 1
      fi
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
  # shellcheck source=/dev/null
  source "${ROOT_DIR}/deploy/stack/lib/claw-region.sh"
  claw_cn_mirror_enabled && CN_FLAG=1
  # shellcheck source=/dev/null
  source "${ROOT_DIR}/deploy/stack/lib/rust-compile-image.sh"
  COMPILE_IMAGE="$(claw_ensure_rust_compile_image "${ROOT_DIR}" "${CONTAINER_CLI}" "${REG}")"
  claw_linux_compile_release "${ROOT_DIR}" "${CONTAINER_CLI}" "${COMPILE_IMAGE}" "${CN_FLAG}"
fi

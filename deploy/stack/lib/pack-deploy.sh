#!/usr/bin/env bash
# 本地标准：打包镜像 + 重启网关栈（macOS 默认 build 走 linux-compile，不在 podman build 里拉 crates.io）
# FC 交互：claw/ttyd 仅来自 e2b 模板；改二进制后需 rebuild template。Author: kejiqing
set -euo pipefail

LIB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
STACK_DIR="$(cd "${LIB_DIR}/.." && pwd)"
ROOT_DIR="$(cd "${STACK_DIR}/../.." && pwd)"

cd "${ROOT_DIR}"

# shellcheck source=/dev/null
source "${LIB_DIR}/claw-step-timing.sh"

if [[ ! -f .env ]]; then
  echo "缺少 .env：cp .env.example .env 并填写" >&2
  exit 1
fi

set -a
# shellcheck source=/dev/null
source "${ROOT_DIR}/.env"
set +a
# shellcheck source=/dev/null
source "${LIB_DIR}/env-profile.sh"
claw_apply_deploy_profile || exit 1
# shellcheck source=/dev/null
source "${LIB_DIR}/release-images.sh"

TAG="local"
BUILD_FLAGS=()
# macOS: slim playground + optional host dist bind-mount. Linux CI/runner: bake admin in image.
if [[ "$(uname -s)" == Darwin ]]; then
  BUILD_FLAGS+=(--skip-playground)
fi
UP_ARGS=()
while [[ $# -gt 0 ]]; do
  case "$1" in
    --clean)
      BUILD_FLAGS=(--clean)
      if [[ "$(uname -s)" == Darwin ]]; then
        BUILD_FLAGS+=(--skip-playground)
      fi
      shift
      ;;
    local | release-*)
      TAG="$1"
      shift
      ;;
    *)
      UP_ARGS+=("$1")
      shift
      ;;
  esac
done

CLAW_TIMING_LABEL="pack-deploy timing"
claw_timing_init

claw_step_begin "1/4 build images (tag=${TAG})"
echo "    日志: deploy/stack/.build.log（全程: tail -f deploy/stack/.build.log）"
echo "    只改 gateway Rust: ./deploy/stack/gateway.sh build local && ./deploy/stack/gateway.sh up" >&2
"${LIB_DIR}/build.sh" "${BUILD_FLAGS[@]}" "${TAG}"

if [[ "${CLAW_INTERACTIVE_BACKEND:-}" == "fc" ]]; then
  claw_apply_pack_deploy_image_tag "${TAG}"
  echo "==> FC: claw/ttyd are baked into e2b template only (no NAS copy)." >&2
  echo "    After claw/ttyd change: python3 deploy/fc-sandbox/build-claw-worker-selfhosted.py && ./deploy/stack/gateway.sh pool-reset" >&2
else
  echo "==> skip FC template hint (CLAW_INTERACTIVE_BACKEND=${CLAW_INTERACTIVE_BACKEND:-unset})"
fi

claw_step_begin "2/4 restart stack (down + up)"
if [[ "${TAG}" == local && -f "${STACK_DIR}/.claw-image-release.env" ]]; then
  echo "==> pack-deploy local: drop sticky release pin ${STACK_DIR}/.claw-image-release.env" >&2
  rm -f "${STACK_DIR}/.claw-image-release.env"
fi
"${LIB_DIR}/down.sh"
if ((${#UP_ARGS[@]} > 0)); then
  "${LIB_DIR}/up.sh" "${UP_ARGS[@]}"
else
  "${LIB_DIR}/up.sh"
fi

claw_step_begin "3/4 stack verify"
"${LIB_DIR}/claw-stack-verify.sh"

claw_step_begin "4/4 connectivity check"
"${LIB_DIR}/check-connectivity.sh"

claw_timing_summary
echo "==> pack-deploy 完成（verify + connectivity 均已通过）"

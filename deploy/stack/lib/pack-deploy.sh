#!/usr/bin/env bash
# 本地标准：打包镜像 + 重启网关栈（macOS 默认 build 走 linux-compile，不在 podman build 里拉 crates.io）
# Author: kejiqing
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

TAG="local"
BUILD_FLAGS=(--no-clean --skip-playground)
UP_ARGS=()
while [[ $# -gt 0 ]]; do
  case "$1" in
    --clean)
      BUILD_FLAGS=(--skip-playground)
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
echo "    日志: deploy/stack/.build.log（另开终端: tail -f deploy/stack/.build.log）"
"${LIB_DIR}/build.sh" "${BUILD_FLAGS[@]}" "${TAG}"

claw_step_begin "2/4 restart stack (down + up)"
"${LIB_DIR}/down.sh" && "${LIB_DIR}/up.sh" ${UP_ARGS+"${UP_ARGS[@]}"}

claw_step_begin "3/4 stack verify"
"${LIB_DIR}/claw-stack-verify.sh"

claw_step_begin "4/4 connectivity check"
"${LIB_DIR}/check-connectivity.sh"

claw_timing_summary
echo "==> pack-deploy 完成（verify + connectivity 均已通过）"

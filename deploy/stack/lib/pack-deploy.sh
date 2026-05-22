#!/usr/bin/env bash
# 本地标准：打包镜像 + 重启网关栈（macOS 默认 build 走 linux-compile，不在 podman build 里拉 crates.io）
# Author: kejiqing
set -euo pipefail

LIB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
STACK_DIR="$(cd "${LIB_DIR}/.." && pwd)"
ROOT_DIR="$(cd "${STACK_DIR}/../.." && pwd)"

cd "${ROOT_DIR}"

if [[ ! -f .env ]]; then
  echo "缺少 .env：cp .env.example .env 并填写" >&2
  exit 1
fi

TAG="${1:-local}"
shift || true

echo "==> 1/2 打包镜像 (tag=${TAG})，日志: deploy/stack/.build.log"
echo "    另开终端: tail -f deploy/stack/.build.log"
"${LIB_DIR}/build.sh" "${TAG}"

echo "==> 2/2 重启部署"
"${LIB_DIR}/down.sh" && "${LIB_DIR}/up.sh" "$@"

echo "==> 完成。连通性检查:"
"${LIB_DIR}/check-connectivity.sh"

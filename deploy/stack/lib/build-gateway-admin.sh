#!/usr/bin/env bash
# Build web/gateway-admin → dist/ (baked into playground image). Author: kejiqing
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
ADMIN_DIR="${ROOT_DIR}/web/gateway-admin"

if [[ "${SKIP_GATEWAY_ADMIN_BUILD:-0}" == "1" ]]; then
  if [[ ! -f "${ADMIN_DIR}/dist/index.html" ]]; then
    echo "SKIP_GATEWAY_ADMIN_BUILD=1 但缺少 ${ADMIN_DIR}/dist/index.html" >&2
    exit 1
  fi
  echo "skip gateway-admin build (SKIP_GATEWAY_ADMIN_BUILD=1)"
  exit 0
fi

claw_admin_node_major() {
  local bin maj
  for bin in node nodejs; do
    if command -v "${bin}" >/dev/null 2>&1; then
      maj="$("${bin}" -p "parseInt(process.versions.node.split('.')[0],10)" 2>/dev/null || echo 0)"
      if [[ "${maj}" =~ ^[0-9]+$ ]] && [[ "${maj}" -gt 0 ]]; then
        echo "${maj}"
        return 0
      fi
    fi
  done
  echo 0
}

claw_admin_container_runtime() {
  command -v podman >/dev/null 2>&1 && printf '%s\n' podman && return 0
  command -v docker >/dev/null 2>&1 && printf '%s\n' docker && return 0
  return 1
}

claw_admin_node_image() {
  local reg="${CONTAINER_BASE_REGISTRY:-docker.1ms.run}"
  reg="${reg%/}"
  printf '%s/library/node:20-alpine' "${reg}"
}

claw_gateway_admin_build_in_container() {
  local rt node_img
  rt="$(claw_admin_container_runtime)" || {
    echo "gateway-admin: 宿主机 Node <18 或未安装 npm，且未找到 podman/docker，无法用容器构建" >&2
    echo "  安装 Node >=18，或安装 podman/docker 后重试" >&2
    return 1
  }
  node_img="$(claw_admin_node_image)"
  echo "==> gateway-admin (container ${node_img}: npm ci && vite build)"
  "${rt}" run --rm \
    -v "${ADMIN_DIR}:/build:rw" \
    -w /build \
    "${node_img}" \
    sh -ec '
      npm config set registry https://registry.npmmirror.com
      if [ -f package-lock.json ]; then npm ci; else npm install; fi
      npm run build
      test -f dist/index.html
      sh -c "test -n \"$(ls dist/assets/*.js 2>/dev/null)\""
    '
}

claw_gateway_admin_build_on_host() {
  echo "==> gateway-admin (host npm ci && vite build)"
  cd "${ADMIN_DIR}"
  if [[ -f package-lock.json ]]; then
    npm ci
  else
    npm install
  fi
  npm run build
}

use_container=0
if [[ "${GATEWAY_ADMIN_BUILD_IN_CONTAINER:-0}" == "1" ]]; then
  use_container=1
elif ! command -v npm >/dev/null 2>&1; then
  use_container=1
else
  maj="$(claw_admin_node_major)"
  if [[ "${maj}" -lt 18 ]]; then
    echo "gateway-admin: 宿主机 Node v${maj} < 18，改用容器构建（可设 GATEWAY_ADMIN_BUILD_IN_CONTAINER=1 强制）" >&2
    use_container=1
  fi
fi

if [[ "${use_container}" == "1" ]]; then
  claw_gateway_admin_build_in_container
else
  claw_gateway_admin_build_on_host
fi

if [[ ! -f "${ADMIN_DIR}/dist/index.html" ]]; then
  echo "gateway-admin build 失败: dist/index.html 不存在" >&2
  exit 1
fi
if ! compgen -G "${ADMIN_DIR}/dist/assets/*.js" >/dev/null; then
  echo "gateway-admin build 失败: dist/assets/*.js 不存在（会导致 /admin 白屏）" >&2
  exit 1
fi
echo "gateway-admin dist: ${ADMIN_DIR}/dist"

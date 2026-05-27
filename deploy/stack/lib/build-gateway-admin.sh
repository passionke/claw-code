#!/usr/bin/env bash
# Local-only: build web/gateway-admin → dist/. Production Admin UI is baked in CI (Containerfile). Author: kejiqing
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
ADMIN_DIR="${ROOT_DIR}/web/gateway-admin"
RELEASE_PIN="${ROOT_DIR}/deploy/stack/.claw-image-release.env"

if [[ "${SKIP_GATEWAY_ADMIN_BUILD:-0}" == "1" ]]; then
  if [[ ! -f "${ADMIN_DIR}/dist/index.html" ]]; then
    echo "SKIP_GATEWAY_ADMIN_BUILD=1 但缺少 ${ADMIN_DIR}/dist/index.html" >&2
    exit 1
  fi
  echo "skip gateway-admin build (SKIP_GATEWAY_ADMIN_BUILD=1)"
  exit 0
fi

if [[ "${CLAW_GATEWAY_ADMIN_LOCAL_BUILD:-0}" != "1" ]]; then
  echo "gateway-admin 不在服务器上编译。" >&2
  echo "  线上：打 tag release-vX.Y.Z → CI 构建 claw-gateway-playground → 服务器 gateway.sh up --release <tag>" >&2
  echo "  本地调试：CLAW_GATEWAY_ADMIN_LOCAL_BUILD=1 ./deploy/stack/gateway.sh admin-build" >&2
  if [[ -f "${RELEASE_PIN}" ]]; then
    echo "  （已存在 ${RELEASE_PIN}，请拉新 CI 镜像，勿 admin-build / admin-reload）" >&2
  fi
  exit 1
fi

if ! command -v npm >/dev/null 2>&1; then
  echo "gateway-admin 需要本机 Node.js/npm（>=18）。镜像内构建由 CI Containerfile 完成。" >&2
  exit 1
fi

maj=0
for bin in node nodejs; do
  if command -v "${bin}" >/dev/null 2>&1; then
    maj="$("${bin}" -p "parseInt(process.versions.node.split('.')[0],10)" 2>/dev/null || echo 0)"
    break
  fi
done
if [[ "${maj}" -lt 18 ]]; then
  echo "gateway-admin 需要 Node >= 18（当前 major=${maj}）。线上请走 CI 镜像。" >&2
  exit 1
fi

echo "==> gateway-admin (local npm ci && vite build)"
cd "${ADMIN_DIR}"
if [[ -f package-lock.json ]]; then
  npm ci
else
  npm install
fi
npm run build

if [[ ! -f dist/index.html ]]; then
  echo "gateway-admin build 失败: dist/index.html 不存在" >&2
  exit 1
fi
if ! compgen -G "dist/assets/*.js" >/dev/null; then
  echo "gateway-admin build 失败: dist/assets/*.js 不存在" >&2
  exit 1
fi
echo "gateway-admin dist: ${ADMIN_DIR}/dist"

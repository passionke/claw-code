#!/usr/bin/env bash
# Build web/gateway-admin → dist/ (committed; baked into playground image). Author: kejiqing
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

if ! command -v npm >/dev/null 2>&1; then
  echo "gateway-admin 需要 Node.js/npm（>=18）。安装后重试，或使用已提交的 web/gateway-admin/dist/" >&2
  exit 1
fi

echo "==> gateway-admin (npm ci && vite build)"
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
echo "gateway-admin dist: ${ADMIN_DIR}/dist"

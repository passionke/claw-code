#!/usr/bin/env bash
# Rebuild gateway-admin dist and load it into running claw-gateway-playground. Author: kejiqing
set -euo pipefail

LIB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${LIB_DIR}/../../.." && pwd)"
DIST="${ROOT_DIR}/web/gateway-admin/dist"
CONTAINER="${CLAW_GATEWAY_PLAYGROUND_CONTAINER:-claw-gateway-playground}"

"${LIB_DIR}/build-gateway-admin.sh"

rt="$(command -v podman 2>/dev/null || command -v docker 2>/dev/null || true)"
if [[ -z "${rt}" ]]; then
  echo "需要 podman 或 docker 将 dist 同步到容器" >&2
  exit 1
fi

if ! "${rt}" container exists "${CONTAINER}" 2>/dev/null; then
  echo "容器 ${CONTAINER} 不存在；请先 ./deploy/stack/gateway.sh up" >&2
  exit 1
fi

"${rt}" cp "${DIST}/." "${CONTAINER}:/app/admin-dist/"
echo "OK — admin dist → ${CONTAINER}:/app/admin-dist"
echo "打开 /admin 后请强制刷新（Cmd+Shift+R）以免浏览器缓存旧 JS"

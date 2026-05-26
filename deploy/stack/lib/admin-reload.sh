#!/usr/bin/env bash
# Rebuild gateway-admin dist; sync into playground when not bind-mounted. Author: kejiqing
set -euo pipefail

LIB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${LIB_DIR}/../../.." && pwd)"
DIST="$(cd "${ROOT_DIR}/web/gateway-admin/dist" && pwd)"
CONTAINER="${CLAW_GATEWAY_PLAYGROUND_CONTAINER:-claw-gateway-playground}"

"${LIB_DIR}/build-gateway-admin.sh"

rt="$(command -v podman 2>/dev/null || command -v docker 2>/dev/null || true)"
if [[ -z "${rt}" ]]; then
  echo "需要 podman 或 docker" >&2
  exit 1
fi

if ! "${rt}" container exists "${CONTAINER}" 2>/dev/null; then
  echo "容器 ${CONTAINER} 不存在；请先 ./deploy/stack/gateway.sh up" >&2
  exit 1
fi

# compose 将 host dist 挂到 /app/admin-dist:ro 时，podman cp 会 500；构建后已生效。
admin_dist_bind_src() {
  "${rt}" inspect "${CONTAINER}" --format '{{range .Mounts}}{{if eq .Destination "/app/admin-dist"}}{{.Source}}{{end}}{{end}}' 2>/dev/null || true
}

bind_src="$(admin_dist_bind_src)"
if [[ -n "${bind_src}" ]]; then
  echo "OK — ${CONTAINER}:/app/admin-dist 已 bind 挂载 ${bind_src}"
  echo "（admin-build 后容器内已是新 dist，无需 cp）"
else
  "${rt}" cp "${DIST}/." "${CONTAINER}:/app/admin-dist/"
  echo "OK — admin dist → ${CONTAINER}:/app/admin-dist"
fi
echo "打开 /admin 后请强制刷新（Cmd+Shift+R）以免浏览器缓存旧 JS"

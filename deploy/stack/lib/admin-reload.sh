#!/usr/bin/env bash
# Local dev only: rebuild admin dist and copy into running playground. Production: use CI image. Author: kejiqing
set -euo pipefail

LIB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${LIB_DIR}/../../.." && pwd)"
DIST="$(cd "${ROOT_DIR}/web/gateway-admin/dist" && pwd)"
CONTAINER="${CLAW_GATEWAY_PLAYGROUND_CONTAINER:-claw-gateway-playground}"

export CLAW_GATEWAY_ADMIN_LOCAL_BUILD=1
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
  echo "（admin-build 后容器内已是新 dist；若未 bind 请 CLAW_GATEWAY_ADMIN_BIND=1 ./deploy/stack/gateway.sh up）"
else
  "${rt}" cp "${DIST}/." "${CONTAINER}:/app/admin-dist/"
  echo "OK — admin dist → ${CONTAINER}:/app/admin-dist"
fi
echo "打开 /admin 后请强制刷新（Cmd+Shift+R）以免浏览器缓存旧 JS"

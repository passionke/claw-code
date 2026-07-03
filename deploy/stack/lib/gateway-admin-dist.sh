#!/usr/bin/env bash
# gateway-admin dist consistency (index.html hashes must match assets/). Author: kejiqing
set -euo pipefail

claw_gateway_admin_dist_dir() {
  local root="${1:-}"
  if [[ -z "${root}" ]]; then
    local here="${BASH_SOURCE[0]:-${0}}"
    root="$(cd "$(dirname "${here}")/../../.." && pwd)"
  fi
  printf '%s/web/gateway-admin/dist' "${root}"
}

# Exit 0 when index.html references exist under dist/assets/.
claw_gateway_admin_dist_consistent() {
  local dist_dir="$1"
  local index="${dist_dir}/index.html"
  if [[ ! -f "${index}" ]]; then
    return 1
  fi
  local rel
  for rel in $(grep -oE 'assets/index-[^"'"'"']+\.(js|css)' "${index}" | sort -u); do
    if [[ ! -f "${dist_dir}/${rel}" ]]; then
      return 1
    fi
  done
  if ! compgen -G "${dist_dir}/assets/*.js" >/dev/null 2>&1; then
    return 1
  fi
  return 0
}

claw_gateway_admin_dist_consistency_hint() {
  local dist_dir="$1"
  local index="${dist_dir}/index.html"
  echo "gateway-admin dist 不完整（index.html 与 assets/ hash 不一致）。" >&2
  echo "  默认：admin 来自 playground 镜像（gateway.sh build / quick 构建完整镜像）。" >&2
  echo "  前端热更新：CLAW_GATEWAY_ADMIN_LOCAL_BUILD=1 ./deploy/stack/gateway.sh admin-build" >&2
  echo "            CLAW_GATEWAY_ADMIN_BIND=1 ./deploy/stack/gateway.sh up" >&2
  if [[ -f "${index}" ]]; then
    echo "  index: $(grep -oE 'assets/index-[^"'"'"']+\.js' "${index}" | head -1 || true)" >&2
    echo "  assets: $(ls "${dist_dir}/assets/"*.js 2>/dev/null | xargs -n1 basename 2>/dev/null | tr '\n' ' ' || echo '<missing>')" >&2
  fi
}

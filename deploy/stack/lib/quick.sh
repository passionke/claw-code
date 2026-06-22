#!/usr/bin/env bash
# 日常本地起栈：host pool-daemon + playground（slim 或已有镜像）+ up + check（不自动 pool-reset）
# Author: kejiqing
set -euo pipefail

LIB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${LIB_DIR}/../../.." && pwd)"
STACK_DIR="${ROOT_DIR}/deploy/stack"

cd "${ROOT_DIR}"

if [[ ! -f .env ]]; then
  echo "缺少 .env：cp .env.example .env 并填写" >&2
  exit 1
fi

set -a
# shellcheck disable=SC1090
source "${ROOT_DIR}/.env"
set +a

echo "==> [1/4] host claw-pool-daemon"
# shellcheck source=/dev/null
source "${LIB_DIR}/pool-daemon-binary.sh"
claw_ensure_pool_daemon_binary "${STACK_DIR}" "${ROOT_DIR}" >/dev/null

echo "==> [2/4] playground image (slim if missing; admin via bind mount when dist/ exists)"
rt="$(command -v podman 2>/dev/null || command -v docker)"
pg_img="${GATEWAY_PLAYGROUND_IMAGE:-claw-gateway-playground:local}"
debian_reg="${CONTAINER_BASE_REGISTRY:-docker.1ms.run}"
debian_reg="${debian_reg%/}"
apt_mirror_arg=(--build-arg "CLAW_USE_CN_APT_MIRROR=0")
[[ "${CLAW_USE_CN_CRATES_MIRROR:-0}" == "1" || "${CLAW_USE_CN_RUST_MIRROR:-0}" == "1" ]] && apt_mirror_arg=(--build-arg "CLAW_USE_CN_APT_MIRROR=1")
if ! "${rt}" image exists "${pg_img}" 2>/dev/null; then
  # shellcheck disable=SC2086
  "${rt}" build -q \
    --build-arg "DEBIAN_BASE_IMAGE=${debian_reg}/library/debian:bookworm-slim" \
    "${apt_mirror_arg[@]}" \
    -f "${ROOT_DIR}/deploy/stack/Containerfile.gateway-playground.slim" \
    -t "${pg_img}" "${ROOT_DIR}" >/dev/null
else
  echo "    reusing ${pg_img}"
fi

echo "==> [3/4] up"
"${LIB_DIR}/up.sh" "$@"

port="${GATEWAY_HOST_PORT:-8088}"
echo "==> [4/4] wait healthz + check"
for _ in $(seq 1 45); do
  if curl -fsS "http://127.0.0.1:${port}/healthz" >/dev/null 2>&1; then
    break
  fi
  sleep 1
done
"${LIB_DIR}/check-connectivity.sh"

pg="${GATEWAY_PLAYGROUND_HOST_PORT:-18765}"
echo ""
echo "OK — gateway http://127.0.0.1:${port}/  playground http://127.0.0.1:${pg}/"

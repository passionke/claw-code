#!/usr/bin/env bash
# 日常本地起栈：host pool-daemon + playground（slim 或已有镜像）+ pool-reset + up + check
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

echo "==> [1/5] host claw-pool-daemon"
# shellcheck source=/dev/null
source "${LIB_DIR}/pool-daemon-binary.sh"
CLAW_POOL_REBUILD_DAEMON=1 claw_ensure_pool_daemon_binary "${STACK_DIR}" "${ROOT_DIR}" >/dev/null

echo "==> [2/5] playground image (slim if missing; admin via bind mount when dist/ exists)"
rt="$(command -v podman 2>/dev/null || command -v docker)"
pg_img="${GATEWAY_PLAYGROUND_IMAGE:-claw-gateway-playground:local}"
if [[ "${GITHUB_ACTIONS:-}" == "true" ]]; then
  py_reg="docker.io"
else
  py_reg="${CONTAINER_BASE_REGISTRY:-docker.1ms.run}"
  py_reg="${py_reg%/}"
fi
if ! "${rt}" image exists "${pg_img}" 2>/dev/null; then
  "${rt}" build -q     --build-arg "PYTHON_BASE_IMAGE=${py_reg}/library/python:3.12-alpine"     -f "${ROOT_DIR}/deploy/stack/Containerfile.gateway-playground.slim"     -t "${pg_img}" "${ROOT_DIR}" >/dev/null
else
  echo "    reusing ${pg_img}"
fi

echo "==> [3/5] pool-reset"
"${LIB_DIR}/pool-reset.sh"

echo "==> [4/5] up"
"${LIB_DIR}/up.sh" "$@"

port="${GATEWAY_HOST_PORT:-8088}"
echo "==> [5/5] wait healthz + check"
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

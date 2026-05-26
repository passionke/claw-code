#!/usr/bin/env bash
# 日常本地起栈：host pool-daemon + 轻量 playground 镜像 + pool-reset + up + check
# Author: kejiqing
set -euo pipefail

LIB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${LIB_DIR}/../../.." && pwd)"

cd "${ROOT_DIR}"

if [[ ! -f .env ]]; then
  echo "缺少 .env：cp .env.example .env 并填写" >&2
  exit 1
fi

set -a
# shellcheck disable=SC1090
source "${ROOT_DIR}/.env"
set +a

echo "==> [1/5] build host claw-pool-daemon (release)"
(cd "${ROOT_DIR}/rust" && cargo build --release -p http-gateway-rs --bin claw-pool-daemon)

echo "==> [2/5] gateway-admin dist + playground image"
"${LIB_DIR}/build-gateway-admin.sh"
rt="$(command -v podman 2>/dev/null || command -v docker)"
pg_img="${GATEWAY_PLAYGROUND_IMAGE:-claw-gateway-playground:local}"
if [[ "${GITHUB_ACTIONS:-}" == "true" ]]; then
  py_reg="docker.io"
else
  py_reg="${CONTAINER_BASE_REGISTRY:-docker.1ms.run}"
  py_reg="${py_reg%/}"
fi
"${rt}" build -q \
  --build-arg "PYTHON_BASE_IMAGE=${py_reg}/library/python:3.12-alpine" \
  -f "${ROOT_DIR}/deploy/stack/Containerfile.gateway-playground" \
  -t "${pg_img}" "${ROOT_DIR}" >/dev/null

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

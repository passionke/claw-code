#!/usr/bin/env bash
# Rust/stack 改动后的唯一发布闭环：打镜像 → up → check。禁止 podman cp / 容器内热编。Author: kejiqing
set -euo pipefail

LIB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PODMAN_DIR="$(cd "${LIB_DIR}/.." && pwd)"
REPO_ROOT="$(cd "${PODMAN_DIR}/../.." && pwd)"

echo "==> claw stack deploy (build → up → check)" >&2
echo "    log: ${PODMAN_DIR}/.build.log" >&2
echo "    do NOT: podman cp host binaries, bare compose up, in-container podman pool" >&2

"${LIB_DIR}/build.sh" "$@"
"${LIB_DIR}/up.sh"
"${LIB_DIR}/check-connectivity.sh"

echo "" >&2
echo "deploy ok. Web: ./deploy/stack/gateway.sh web-ui  then open :4100" >&2

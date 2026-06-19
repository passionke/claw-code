#!/usr/bin/env bash
# Local-only: build web/claw-display → dist/. Author: kejiqing
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
DISPLAY_DIR="${ROOT_DIR}/web/claw-display"
OUT_DIR="${ROOT_DIR}/web/gateway-async-playground/claw-display"

if [[ "${SKIP_CLAW_DISPLAY_BUILD:-0}" == "1" ]]; then
  if [[ ! -f "${OUT_DIR}/claw-display.js" ]]; then
    echo "SKIP_CLAW_DISPLAY_BUILD=1 但缺少 ${OUT_DIR}/claw-display.js" >&2
    exit 1
  fi
  echo "skip claw-display build (SKIP_CLAW_DISPLAY_BUILD=1)"
  exit 0
fi

if [[ "${CLAW_DISPLAY_LOCAL_BUILD:-0}" != "1" ]]; then
  echo "claw-display 不在服务器上编译。" >&2
  echo "  本地调试：CLAW_DISPLAY_LOCAL_BUILD=1 ./deploy/stack/gateway.sh claw-display-build" >&2
  if [[ ! -f "${OUT_DIR}/claw-display.js" ]]; then
    echo "  缺少 ${OUT_DIR}/claw-display.js — 请先本地 build 并提交产物。" >&2
    exit 1
  fi
  exit 0
fi

if ! command -v npm >/dev/null 2>&1; then
  echo "claw-display 需要本机 Node.js/npm（>=18）。" >&2
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
  echo "claw-display 需要 Node >= 18（当前 major=${maj}）。" >&2
  exit 1
fi

echo "==> claw-display (local npm ci && vite build)"
cd "${DISPLAY_DIR}"
if [[ -f package-lock.json ]]; then
  npm ci
else
  npm install
fi
npm run build

if [[ ! -f dist/claw-display.js ]]; then
  echo "claw-display build 失败: dist/claw-display.js 不存在" >&2
  exit 1
fi

mkdir -p "${OUT_DIR}"
cp -f dist/claw-display.js "${OUT_DIR}/"
if [[ -f dist/style.css ]]; then
  cp -f dist/style.css "${OUT_DIR}/claw-display.css"
fi
echo "claw-display copied: ${OUT_DIR}/"

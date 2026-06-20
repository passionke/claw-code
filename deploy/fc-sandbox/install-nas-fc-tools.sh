#!/usr/bin/env bash
# One-time: stage claw + ttyd + claude-tap on NAS for FC sandbox bootstrap. Author: kejiqing
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "${ROOT_DIR}"

if [[ -f "${ROOT_DIR}/.env" ]]; then
  set -a
  # shellcheck source=/dev/null
  source "${ROOT_DIR}/.env"
  set +a
fi

WORKER_IMAGE="${CLAW_FC_WORKER_IMAGE:-crpi-cf9vxpq3n8or17mw.cn-hangzhou.personal.cr.aliyuncs.com/passionke/claw-gateway-worker:release-v1.6.12}"
TOOLS_REL="${CLAW_FC_NAS_TOOLS_REL:-.claw-fc-tools}"
RT="${CLAW_CONTAINER_RUNTIME:-podman}"
if [[ "${RT}" == "auto" ]]; then
  RT="podman"
fi

# NAS mount on gateway host (234 ECS) or local work_root bind.
if [[ -n "${CLAW_NAS_HOST_MOUNT:-}" ]]; then
  NAS_ROOT="${CLAW_NAS_HOST_MOUNT}"
elif [[ -n "${CLAW_POOL_WORK_ROOT_BIND_SRC:-}" ]]; then
  NAS_ROOT="${CLAW_POOL_WORK_ROOT_BIND_SRC}"
else
  NAS_ROOT="${ROOT_DIR}/deploy/stack/claw-workspace"
fi

DEST="${NAS_ROOT}/${TOOLS_REL}"
TMP="${ROOT_DIR}/deploy/fc-sandbox/.fc-tools-staging"

echo "==> NAS tools dir: ${DEST}"
echo "==> worker image: ${WORKER_IMAGE}"

mkdir -p "${TMP}" "${DEST}"
"${RT}" pull "${WORKER_IMAGE}" 2>/dev/null || true
cid="$("${RT}" create "${WORKER_IMAGE}")"
trap '"${RT}" rm -f "${cid}"' EXIT
"${RT}" cp "${cid}:/usr/local/bin/claw" "${TMP}/claw"
"${RT}" cp "${cid}:/usr/local/bin/ttyd" "${TMP}/ttyd"
chmod +x "${TMP}/claw" "${TMP}/ttyd"

# Optional: linux/amd64 claude-tap runtime bundle for cloud FC (NAS bootstrap).
TAP_IMAGE="${CLAUDE_TAP_IMAGE:-crpi-cf9vxpq3n8or17mw.cn-hangzhou.personal.cr.aliyuncs.com/passionke/claw-tap:latest}"
TAP_RT="${TMP}/tap-runtime"
if "${RT}" pull --platform linux/amd64 "${TAP_IMAGE}" 2>/dev/null; then
  tap_cid="$("${RT}" create --platform linux/amd64 "${TAP_IMAGE}")"
  mkdir -p "${TAP_RT}/bin" "${TAP_RT}/lib"
  "${RT}" cp "${tap_cid}:/usr/local/bin/python3.12" "${TAP_RT}/bin/" 2>/dev/null || true
  "${RT}" cp "${tap_cid}:/usr/local/bin/claude-tap" "${TAP_RT}/bin/" 2>/dev/null || true
  "${RT}" cp "${tap_cid}:/usr/local/lib/python3.12/site-packages" "${TAP_RT}/lib/" 2>/dev/null || true
  "${RT}" rm -f "${tap_cid}" 2>/dev/null || true
  chmod +x "${TAP_RT}/bin/"* 2>/dev/null || true
fi

install -m 755 "${TMP}/claw" "${DEST}/claw"
install -m 755 "${TMP}/ttyd" "${DEST}/ttyd"
if [[ -d "${TAP_RT}/bin" ]] && [[ -f "${TAP_RT}/bin/claude-tap" ]]; then
  rm -rf "${DEST}/tap-runtime"
  cp -a "${TAP_RT}" "${DEST}/tap-runtime"
fi
rm -rf "${TMP}"

ls -la "${DEST}/"
echo "OK: FC NAS tools installed under ${DEST}"
echo "Set CLAW_FC_TEMPLATE=code-interpreter-v1 and CLAW_FC_NAS_VOLUME_NAME in .env"

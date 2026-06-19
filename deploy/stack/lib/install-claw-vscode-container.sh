#!/usr/bin/env bash
# Package claw-vscode + install into running OVS via openvscode-server --install-extension. Author: kejiqing
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
# shellcheck source=/dev/null
[[ -f "${ROOT_DIR}/.env" ]] && source "${ROOT_DIR}/.env"

CONTAINER="${CLAW_OVS_CONTAINER:-claw-openvscode-server}"
EXT_DIR="/opt/claw-extensions"
SD="/opt/claw-ovs/server-data"
OVS_BIN="/home/.openvscode-server/bin/openvscode-server"
EXT_VER="$(python3 -c "import json; print(json.load(open('${ROOT_DIR}/extensions/claw-vscode/package.json'))['version'])")"
VSIX_HOST="${ROOT_DIR}/deploy/stack/claw.claw-vscode-${EXT_VER}.vsix"
WORKSPACE="${CLAW_POOL_WORK_ROOT_BIND_SRC:-${ROOT_DIR}/deploy/stack/claw-workspace}"

fail() { echo "install-claw-vscode-container: $*" >&2; exit 1; }

echo "==> package VSIX"
chmod +x "${ROOT_DIR}/deploy/stack/lib/package-ovs-extension-vsix.sh"
"${ROOT_DIR}/deploy/stack/lib/package-ovs-extension-vsix.sh" \
  "${ROOT_DIR}/extensions/claw-vscode" \
  "${VSIX_HOST}"

echo "==> workspace settings (gatewayHost only; projId from Gateway proj_N/home/.vscode)"
mkdir -p "${WORKSPACE}/.vscode"
python3 - "${WORKSPACE}/.vscode/settings.json" <<'PY'
import json, sys
dst = sys.argv[1]
cfg = {}
try:
    with open(dst, encoding="utf-8") as f:
        cfg = json.load(f)
except FileNotFoundError:
    pass
cfg["claw.gatewayHost"] = "gateway-rs:8080"
cfg.pop("claw.projId", None)
with open(dst, "w", encoding="utf-8") as f:
    json.dump(cfg, f, indent=2, ensure_ascii=False)
    f.write("\n")
print(f"wrote {dst}")
PY

podman container exists "${CONTAINER}" >/dev/null 2>&1 || fail "container ${CONTAINER} not running"

echo "==> install-extension in ${CONTAINER} (HOME=/opt/claw-ovs/home)"
podman cp "${VSIX_HOST}" "${CONTAINER}:/tmp/claw-vscode.vsix"
podman exec -e HOME=/opt/claw-ovs/home "${CONTAINER}" "${OVS_BIN}" \
  --install-extension /tmp/claw-vscode.vsix \
  --extensions-dir="${EXT_DIR}" \
  --server-data-dir="${SD}" \
  --force 2>/dev/null | tail -1

podman exec -e HOME=/opt/claw-ovs/home "${CONTAINER}" "${OVS_BIN}" \
  --list-extensions \
  --extensions-dir="${EXT_DIR}" \
  --server-data-dir="${SD}" 2>/dev/null \
  | grep -q '^claw\.claw-vscode$' || fail "claw.claw-vscode not listed after install-extension"

echo "==> syntax check (OVS Node)"
podman exec "${CONTAINER}" /home/.openvscode-server/node --check \
  "${EXT_DIR}/claw.claw-vscode-${EXT_VER}/extension.js" \
  || fail "extension.js syntax error in container"

echo "==> restart ${CONTAINER}"
podman restart "${CONTAINER}" >/dev/null
PORT="${CLAW_OVS_HOST_PORT:-13000}"
for i in $(seq 1 45); do
  code=$(curl -sS -o /dev/null -w '%{http_code}' "http://127.0.0.1:${PORT}/ovs/?folder=/home/workspace/proj_1/home" 2>/dev/null || echo 000)
  [[ "${code}" == "200" ]] && break
  sleep 1
done
[[ "${code:-000}" == "200" ]] || fail "OVS not ready on :${PORT}/ovs/ (last ${code:-000})"

NET="${CLAW_PODMAN_NETWORK:-${COMPOSE_PROJECT_NAME:-claw}_default}"
if ! podman exec "${CONTAINER}" /home/.openvscode-server/node -e \
  "require('dns').lookup('gateway-rs',(e)=>process.exit(e?1:0));" 2>/dev/null; then
  echo "==> connect ${CONTAINER} → ${NET} (gateway-rs DNS for @claw agent WS)"
  podman network connect "${NET}" "${CONTAINER}" 2>/dev/null \
    || fail "OVS cannot resolve gateway-rs; run: ./deploy/stack/gateway.sh up"
fi

echo "OK: claw.claw-vscode-${EXT_VER} installed"

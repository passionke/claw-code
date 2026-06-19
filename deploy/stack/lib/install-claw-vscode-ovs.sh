#!/usr/bin/env bash
# Package claw-vscode from claw-code and install into local openvscode-server. Author: kejiqing
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
# shellcheck source=/dev/null
[[ -f "${ROOT_DIR}/.env" ]] && source "${ROOT_DIR}/.env"
OVS_ROOT="${CLAW_OVS_ROOT:-${HOME}/work/openvscode-server}"
ARCH="$(uname -m)"
case "${ARCH}" in
  arm64) VSCODE_ARCH=arm64; PLATFORM=darwin ;;
  x86_64) VSCODE_ARCH=x64; PLATFORM=darwin ;;
  aarch64) VSCODE_ARCH=arm64; PLATFORM=linux ;;
  *) VSCODE_ARCH=x64; PLATFORM=linux ;;
esac

BUILD_ROOT="$(dirname "${OVS_ROOT}")"
DEFAULT_OVS_BIN="${BUILD_ROOT}/vscode-reh-web-${PLATFORM}-${VSCODE_ARCH}/bin/openvscode-server"
OVS_BIN="${CLAW_OVS_BIN:-${DEFAULT_OVS_BIN}}"
EXT_DIR="${CLAW_OVS_EXT_DIR:-${OVS_ROOT}/.build/ovs-extensions}"
SD="${CLAW_OVS_SD:-${OVS_ROOT}/.build/ovs-server-data}"
VSIX="${CLAW_OVS_VSIX:-${ROOT_DIR}/deploy/stack/claw.claw-vscode-0.2.0.vsix}"
SETTINGS_SRC="${ROOT_DIR}/deploy/stack/openvscode-settings.json"
PLAYGROUND_PORT="${GATEWAY_PLAYGROUND_HOST_PORT:-18765}"

fail() { echo "install-claw-vscode-ovs: $*" >&2; exit 1; }

[[ -x "${OVS_BIN}" ]] || fail "OVS binary not found: ${OVS_BIN} (set CLAW_OVS_BIN or build OVS first)"
[[ -f "${SETTINGS_SRC}" ]] || fail "missing ${SETTINGS_SRC}"

echo "==> package VSIX"
chmod +x "${ROOT_DIR}/deploy/stack/lib/package-ovs-extension-vsix.sh"
"${ROOT_DIR}/deploy/stack/lib/package-ovs-extension-vsix.sh" \
  "${ROOT_DIR}/extensions/claw-vscode" \
  "${VSIX}"

echo "==> merge Machine settings"
mkdir -p "${SD}/Machine" "${EXT_DIR}"
GATEWAY_HOST="${GATEWAY_HOST_PORT:-8088}"
python3 - "${SETTINGS_SRC}" "${SD}/Machine/settings.json" "${GATEWAY_HOST}" <<'PY'
import json, sys
src, dst, gw_port = sys.argv[1], sys.argv[2], sys.argv[3]
with open(src, encoding="utf-8") as f:
    cfg = json.load(f)
cfg["claw.gatewayHost"] = f"127.0.0.1:{gw_port}"
cfg.pop("claw.agentWsBase", None)
with open(dst, "w", encoding="utf-8") as f:
    json.dump(cfg, f, indent=2, ensure_ascii=False)
    f.write("\n")
print(f"wrote {dst}")
PY

WS_DIR="${OVS_ROOT}/.build/ovs-workspace/.vscode"
mkdir -p "${WS_DIR}"
GATEWAY_HOST="${GATEWAY_HOST_PORT:-8088}"
python3 - "${WS_DIR}/settings.json" "${GATEWAY_HOST}" <<'PY'
import json, sys
dst, gw_port = sys.argv[1], sys.argv[2]
cfg = {"claw.gatewayHost": f"127.0.0.1:{gw_port}"}
with open(dst, "w", encoding="utf-8") as f:
    json.dump(cfg, f, indent=2, ensure_ascii=False)
    f.write("\n")
print(f"wrote workspace {dst}")
PY

echo "==> install-extension"
HOME="${CLAW_OVS_HOME:-${OVS_ROOT}/.build/ovs-home}"
mkdir -p "${HOME}"
export HOME
"${OVS_BIN}" \
  --install-extension "${VSIX}" \
  --extensions-dir="${EXT_DIR}" \
  --server-data-dir="${SD}" \
  --force

echo "==> list-extensions"
"${OVS_BIN}" \
  --list-extensions \
  --extensions-dir="${EXT_DIR}" \
  --server-data-dir="${SD}" \
  | grep -q '^claw\.claw-vscode$' || fail "claw.claw-vscode not listed after install"

echo "OK: claw.claw-vscode installed"
echo "Restart OVS with: --enable-proposed-api=claw.claw-vscode"
echo "  e.g. bash ${OVS_ROOT}/scripts/ovs-chat/run-claw-dev.sh"
echo "Backend: ./deploy/stack/gateway.sh quick  (Playground :${PLAYGROUND_PORT})"

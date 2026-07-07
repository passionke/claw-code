#!/usr/bin/env bash
# Apply repo-root .env infra anchors → co-located e2bserver panel/worker config + optional nginx traffic.
# Author: kejiqing
#
# Usage:
#   ./deploy/stack/gateway.sh sync-e2b-env              # patch configs only
#   ./deploy/stack/gateway.sh sync-e2b-env --restart    # patch + restart panel/worker
#   ./deploy/stack/gateway.sh sync-e2b-env --nginx      # patch + install/reload nginx traffic (:80→:3001)
set -euo pipefail

LIB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${LIB_DIR}/../../.." && pwd)"
ENV_FILE="${REPO_ROOT}/.env"

do_restart=0
do_nginx=0
for arg in "$@"; do
  case "${arg}" in
    --restart) do_restart=1 ;;
    --nginx) do_nginx=1 ;;
  esac
done

if [[ ! -f "${ENV_FILE}" ]]; then
  echo "error: missing ${ENV_FILE}" >&2
  exit 1
fi

set -a
# shellcheck disable=SC1090
source "${ENV_FILE}"
set +a

E2B_ROOT="${CLAW_E2B_SERVER_ROOT:-}"
if [[ -z "${E2B_ROOT}" ]]; then
  echo "error: set CLAW_E2B_SERVER_ROOT in .env (e2bserver checkout on this host)" >&2
  exit 1
fi
if [[ ! -d "${E2B_ROOT}" ]]; then
  echo "error: CLAW_E2B_SERVER_ROOT=${E2B_ROOT} is not a directory" >&2
  exit 1
fi

: "${CLAW_E2B_HOST:?set CLAW_E2B_HOST in .env}"
: "${CLAW_E2B_DOMAIN:?set CLAW_E2B_DOMAIN in .env}"
: "${CLAW_E2B_NAS_HOST_MOUNT:?set CLAW_E2B_NAS_HOST_MOUNT in .env}"

PANEL_CFG="${CLAW_E2B_PANEL_CONFIG:-${E2B_ROOT}/config/deploy.toml}"
WORKER_CFG="${CLAW_E2B_WORKER_CONFIG:-${E2B_ROOT}/config/worker.toml}"
TRAFFIC_PORT="${CLAW_E2B_TRAFFIC_PORT:-3001}"

patch_toml_scalar() {
  local file="$1" key="$2" val="$3"
  [[ -f "${file}" ]] || { echo "error: missing ${file}" >&2; return 1; }
  python3 - "${file}" "${key}" "${val}" <<'PY'
import re, sys
path, key, val = sys.argv[1:4]
text = open(path, encoding="utf-8").read()
pat = re.compile(rf'^{re.escape(key)}\s*=\s*.*$', re.M)
repl = f'{key} = "{val}"'
if pat.search(text):
    text = pat.sub(repl, text, count=1)
else:
    text = text.rstrip() + "\n" + repl + "\n"
open(path, "w", encoding="utf-8").write(text)
PY
}

patch_toml_nas_block() {
  local file="$1" mount_root="$2" nas_server="${3:-}" nas_export="${4:-}"
  [[ -f "${file}" ]] || { echo "error: missing ${file}" >&2; return 1; }
  python3 - "${file}" "${mount_root}" "${nas_server}" "${nas_export}" <<'PY'
import re, sys
path, mount_root, nas_server, nas_export = sys.argv[1:5]
text = open(path, encoding="utf-8").read()

def sub_key(src, k, v):
    pat = re.compile(rf'^(?P<indent>\s*){re.escape(k)}\s*=\s*.*$', re.M)
    m = pat.search(src)
    if m:
        return pat.sub(f'{m.group("indent")}{k} = "{v}"', src, count=1)
    return src

if "[nas]" not in text:
    text = text.rstrip() + f'''

[nas]
server = "{nas_server}"
export = "{nas_export}"
host_mount_root = "{mount_root}"
nfs_version = "3"
sandbox_inject = "bind"
'''
else:
    text = sub_key(text, "server", nas_server)
    text = sub_key(text, "export", nas_export)
    text = sub_key(text, "host_mount_root", mount_root)
open(path, "w", encoding="utf-8").write(text)
PY
}

stamp_synced() {
  local file="$1"
  local marker="# synced from claw-code .env ($(date -Iseconds)) — Author: kejiqing"
  if grep -q '^# synced from claw-code .env' "${file}" 2>/dev/null; then
    sed -i "s|^# synced from claw-code .env.*|${marker}|" "${file}"
  else
    sed -i "1s|^|${marker}\n|" "${file}"
  fi
}

NAS_SERVER="${CLAW_E2B_NAS_SERVER:-}"
NAS_EXPORT="${CLAW_E2B_NAS_EXPORT:-}"
if [[ "${CLAW_USE_NAS_VOLUME:-0}" == "0" && -z "${NAS_SERVER}" ]]; then
  NAS_SERVER=""
  NAS_EXPORT=""
fi

assert_dns_ready() {
  local domain="${CLAW_E2B_DOMAIN:-}"
  local host="${CLAW_E2B_HOST:-}"
  [[ -n "${domain}" && -n "${host}" ]] || return 0
  [[ "${domain}" == "localhost" ]] && return 0
  if getent ahosts "${domain}" 2>/dev/null | awk '{print $1}' | grep -qx "${host}"; then
    return 0
  fi
  echo "error: DNS not ready: ${domain} does not resolve to ${host}" >&2
  echo "hint: fix DNS apex/wildcard first, then rerun sync-e2b-env" >&2
  exit 1
}

echo "==> sync e2b host env from ${ENV_FILE}" >&2
echo "    e2b root: ${E2B_ROOT}" >&2
echo "    sandbox_domain=${CLAW_E2B_DOMAIN} host_mount_root=${CLAW_E2B_NAS_HOST_MOUNT}" >&2

assert_dns_ready

patch_toml_scalar "${PANEL_CFG}" "sandbox_domain" "${CLAW_E2B_DOMAIN}"
patch_toml_nas_block "${PANEL_CFG}" "${CLAW_E2B_NAS_HOST_MOUNT}" "${NAS_SERVER}" "${NAS_EXPORT}"
stamp_synced "${PANEL_CFG}"

if [[ -f "${WORKER_CFG}" ]]; then
  patch_toml_nas_block "${WORKER_CFG}" "${CLAW_E2B_NAS_HOST_MOUNT}" "${NAS_SERVER}" "${NAS_EXPORT}"
  stamp_synced "${WORKER_CFG}"
fi

if [[ "${do_nginx}" == "1" || "${CLAW_E2B_TRAFFIC_NGINX:-0}" == "1" ]]; then
  NGINX_SCRIPT="${E2B_ROOT}/scripts/install-nginx-traffic.sh"
  if [[ -x "${NGINX_SCRIPT}" ]] || [[ -f "${NGINX_SCRIPT}" ]]; then
    echo "==> nginx traffic proxy (:80 → :${TRAFFIC_PORT})" >&2
    bash "${NGINX_SCRIPT}"
  else
    echo "warn: ${NGINX_SCRIPT} missing; skip nginx traffic install" >&2
  fi
fi

if [[ "${do_restart}" == "1" ]]; then
  echo "==> restart e2b panel + worker" >&2
  (cd "${E2B_ROOT}" && E2B_CONFIG="${PANEL_CFG}" ./scripts/start-panel.sh)
  if [[ -f "${E2B_ROOT}/scripts/start-worker.sh" ]]; then
    (cd "${E2B_ROOT}" && E2B_WORKER_CONFIG="${WORKER_CFG}" ./scripts/start-worker.sh)
  fi
fi

echo "OK: e2b panel/worker config synced from .env" >&2
echo "    panel: ${PANEL_CFG}" >&2
echo "    worker: ${WORKER_CFG}" >&2
if [[ "${do_restart}" != "1" ]]; then
  echo "hint: after .env change run: ./deploy/stack/gateway.sh sync-e2b-env --restart" >&2
fi

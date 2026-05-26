#!/usr/bin/env bash
# One-click deploy: GHCR claw-code + claw-gateway-worker at a release tag; claude-tap @ latest.
# Preserves business keys in repo-root `.env` / `.claw.json` — only GATEWAY_IMAGE / CLAW_DOCKER_IMAGE
# (and optional CLAW_PODMAN_IMAGE if present) are rewritten to match the tag.
#
# Usage (on server, repo at /home/admin/claw-code):
#   ./deploy/server-252/deploy-one-click.sh release-v1.0.17
#   GHCR_OWNER=ultraworkers ./deploy/server-252/deploy-one-click.sh release-v1.0.17
#
# Author: kejiqing
set -euo pipefail

export LC_ALL="${LC_ALL:-C.UTF-8}"
export LANG="${LANG:-C.UTF-8}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
PODMAN_DIR="${REPO_ROOT}/deploy/podman"
ENV_FILE="${REPO_ROOT}/.env"

die() {
  echo "error: $*" >&2
  exit 1
}

[[ -f "${ENV_FILE}" ]] || die "missing ${ENV_FILE} — copy .env.example and configure (do not remove your business vars)."

[[ "${#}" -ge 1 ]] || die "usage: $0 <release-tag|version> [ghcr_owner]  e.g. $0 release-v1.0.17"

RAW_TAG="$1"
if [[ "${RAW_TAG}" == release-* ]]; then
  RELEASE_TAG="${RAW_TAG}"
else
  RELEASE_TAG="release-${RAW_TAG}"
fi

GHCR_OWNER="${2:-${GHCR_OWNER:-passionke}}"
GATEWAY_IMAGE="ghcr.io/${GHCR_OWNER}/claw-code:${RELEASE_TAG}"
WORKER_IMAGE="ghcr.io/${GHCR_OWNER}/claw-gateway-worker:${RELEASE_TAG}"

upsert_env_kv() {
  local key="$1"
  local val="$2"
  python3 - "${ENV_FILE}" "${key}" "${val}" <<'PY'
import re
import sys

path, key, val = sys.argv[1], sys.argv[2], sys.argv[3]


def fmt_line(k: str, v: str) -> str:
    # So `source .env` does not treat `--env-file /path` as command + argv. Author: kejiqing
    if re.search(r"[\s#'\"]", v) or v.startswith("-"):
        q = "'" + v.replace("'", "'\"'\"'") + "'"
        return f"{k}={q}\n"
    return f"{k}={v}\n"


with open(path, encoding="utf-8") as f:
    lines = f.readlines()
out, seen = [], False
for line in lines:
    if line.startswith(f"{key}="):
        if not seen:
            out.append(fmt_line(key, val))
            seen = True
        continue
    out.append(line)
if not seen:
    out.append(fmt_line(key, val))
with open(path, "w", encoding="utf-8") as f:
    f.writelines(out)
PY
}

echo "==> updating image lines in .env (other keys untouched)"
upsert_env_kv "GATEWAY_IMAGE" "${GATEWAY_IMAGE}"
upsert_env_kv "CLAW_DOCKER_IMAGE" "${WORKER_IMAGE}"
if grep -q '^CLAW_PODMAN_IMAGE=' "${ENV_FILE}" 2>/dev/null; then
  upsert_env_kv "CLAW_PODMAN_IMAGE" "${WORKER_IMAGE}"
fi

echo "==> ensuring gateway + pool + tap routing keys in .env (append only if missing)"
python3 - "${ENV_FILE}" <<'PY'
import re
import sys

path = sys.argv[1]
with open(path, encoding="utf-8") as f:
    text = f.read()

def has_key(k: str) -> bool:
    return re.search(rf"^{re.escape(k)}=", text, re.M) is not None

defaults = [
    ("GATEWAY_HOST_PORT", "18088"),
    ("CLAW_TIMEOUT_SECONDS", "120"),
    ("CLAW_SOLVE_ISOLATION", "docker_pool"),
    ("CLAW_POOL_HOST_DAEMON", "1"),
    ("CLAW_POOL_DAEMON_SKIP_BUILD", "1"),
    ("CLAW_POOL_DAEMON_TCP_HOST", "host.docker.internal"),
    ("CLAW_CONTAINER_RUNTIME", "docker"),
    ("CLAW_DOCKER_POOL_SIZE", "4"),
    ("CLAW_DOCKER_POOL_MIN_IDLE", "1"),
    ("CLAW_HOST_LOG_DIR", "./deploy/podman/claw-logs"),
    ("CLAUDE_TAP_MODE", "docker"),
    ("CLAUDE_TAP_IMAGE", "ghcr.io/passionke/claude-tap:latest"),
    # Published host port for claude-tap (compose left side); same port in OPENAI_BASE_URL for claw-code.
    ("CLAUDE_TAP_HOST_PORT", "8080"),
    ("CLAUDE_TAP_LIVE_PORT", "3000"),
    # Gateway/worker reach tap via this host (Linux Docker: host.docker.internal).
    ("CLAUDE_TAP_BIND_HOST", "host.docker.internal"),
]
missing = [(k, v) for k, v in defaults if not has_key(k)]
if missing:
    with open(path, "a", encoding="utf-8") as f:
        f.write("\n# deploy-one-click: gateway pool defaults (kejiqing)\n")
        for k, v in missing:
            f.write(f"{k}={v}\n")
    with open(path, encoding="utf-8") as f:
        text = f.read()
PY

echo "==> linking claw-code OPENAI_* to claude-tap ports (refresh-tap-llm-chain-in-env.py)"
python3 "${PODMAN_DIR}/refresh-tap-llm-chain-in-env.py" "${ENV_FILE}"

echo "==> syncing deploy/podman/worker-openai.env (run same script alone on server: deploy/podman/sync-worker-openai-env.sh)"
"${PODMAN_DIR}/sync-worker-openai-env.sh"

# shellcheck source=/dev/null
source "${PODMAN_DIR}/compose-include.sh"
RT="$(claw_container_runtime_cli)" || die "need docker or podman in PATH"

echo "==> pulling images (${RT})"
"${RT}" pull "${GATEWAY_IMAGE}"
"${RT}" pull "${WORKER_IMAGE}"
set -a
# shellcheck source=/dev/null
source "${ENV_FILE}"
set +a
if [[ "${CLAUDE_TAP_MODE:-docker}" != "host" ]]; then
  TAP_IMG="${CLAUDE_TAP_IMAGE:-ghcr.io/passionke/claude-tap:latest}"
  echo "==> pulling claude-tap image (${TAP_IMG})"
  "${RT}" pull "${TAP_IMG}"
fi

echo "==> stopping prior gateway, pool daemon, claude-tap (deploy/podman/stop-with-tap.sh)"
"${PODMAN_DIR}/stop-with-tap.sh" || true
if [[ "${CLAUDE_TAP_MODE:-docker}" == "host" ]]; then
  pkill -f 'claude-tap.*--tap-no-launch' 2>/dev/null || true
fi
sleep 0.5

echo "==> removing stray claw containers (historical workers / gateway)"
mapfile -t STALE < <("${RT}" ps -aq --filter "name=claw-" 2>/dev/null || true)
if [[ "${#STALE[@]}" -gt 0 ]]; then
  "${RT}" rm -f "${STALE[@]}" || true
fi

echo "==> removing legacy compose names (e.g. old docker claude-tap / claw-code)"
for c in claude-tap claw-code; do
  if "${RT}" inspect "${c}" >/dev/null 2>&1; then
    "${RT}" rm -f "${c}" || true
  fi
done

echo "==> installing host claw-pool-daemon from GATEWAY_IMAGE"
if [[ "$(id -u)" -eq 0 ]]; then
  POOL_OUT="${CLAW_POOL_DAEMON_BIN:-/usr/local/bin/claw-pool-daemon}"
else
  POOL_OUT="${CLAW_POOL_DAEMON_BIN:-${HOME}/.local/bin/claw-pool-daemon}"
fi
mkdir -p "$(dirname "${POOL_OUT}")"
"${PODMAN_DIR}/install-pool-daemon-from-image.sh" "${POOL_OUT}"
if [[ "$(id -u)" -ne 0 ]]; then
  upsert_env_kv "CLAW_POOL_DAEMON_BIN" "${POOL_OUT}"
  upsert_env_kv "CLAW_POOL_DAEMON_SKIP_BUILD" "1"
fi

if [[ "${CLAUDE_TAP_MODE:-docker}" == "host" ]]; then
  echo "==> upgrading claude-tap (pip/uv; CLAUDE_TAP_MODE=host)"
  if command -v uv >/dev/null 2>&1; then
    uv tool install claude-tap --force
  elif command -v pip3 >/dev/null 2>&1; then
    pip3 install -U claude-tap
  elif command -v python3 >/dev/null 2>&1; then
    python3 -m pip install -U claude-tap
  else
    die "install uv or pip for host claude-tap, or set CLAUDE_TAP_MODE=docker"
  fi
else
  echo "==> claude-tap: using container (\${CLAUDE_TAP_IMAGE}); skip pip"
fi

echo "==> starting claude-tap + gateway (pooled host daemon)"
"${PODMAN_DIR}/start-with-tap.sh"

set -a
# shellcheck source=/dev/null
source "${ENV_FILE}"
set +a
PORT="${GATEWAY_HOST_PORT:-8088}"
HEALTH_URL="http://127.0.0.1:${PORT}/healthz"

echo "==> health check: ${HEALTH_URL}"
BODY=""
for _ in $(seq 1 60); do
  if BODY="$(curl -fsS "${HEALTH_URL}" 2>/dev/null)"; then
    if echo "${BODY}" | python3 -c "import json,sys; j=json.load(sys.stdin); sys.exit(0 if j.get('ok') else 1)" 2>/dev/null; then
      break
    fi
  fi
  BODY=""
  sleep 1
done

[[ -n "${BODY}" ]] || die "healthz never became ready"
echo "${BODY}"
echo "${BODY}" | python3 -c "import json,sys; j=json.load(sys.stdin); assert j.get('containerPool'), 'containerPool must be true (pool mode)'; assert j.get('poolRpcRemote'), 'poolRpcRemote must be true (host daemon RPC)'" \
  || die "/healthz: pool checks failed (see message above)"
echo "OK: pooled gateway up (${RELEASE_TAG})."

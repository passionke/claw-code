#!/usr/bin/env bash
# Refresh claude-tap → OPENAI_BASE_URL in repo-root .env and normalize pool worker `run` extras.
# Worker LLM/MCP keys are NOT copied here: pool mounts root `.env` and `claw gateway-solve-once`
# loads declared keys via `gateway_solve_turn::apply_worker_env` (see worker_env.rs).
#
# `pool-daemon-up.sh` calls this before daemon start. Manual:
#   ./deploy/stack/lib/sync-worker-openai-env.sh [--restart]
#
# Author: kejiqing
set -euo pipefail

export LC_ALL="${LC_ALL:-C.UTF-8}"
export LANG="${LANG:-C.UTF-8}"

LIB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PODMAN_DIR="$(cd "${LIB_DIR}/.." && pwd)"
REPO_ROOT="$(cd "${PODMAN_DIR}/../.." && pwd)"
ENV_FILE="${REPO_ROOT}/.env"

die() {
  echo "error: $*" >&2
  exit 1
}

[[ -f "${ENV_FILE}" ]] || die "missing ${ENV_FILE}"

echo "==> refresh claw-code LLM URL vs claude-tap ports (${ENV_FILE})"
REFRESH_PY="${PODMAN_DIR}/refresh-tap-llm-chain-in-env.py"
[[ -f "${REFRESH_PY}" ]] || die "missing ${REFRESH_PY}"
python3 "${REFRESH_PY}" "${ENV_FILE}"

upsert_env_kv() {
  local key="$1"
  local val="$2"
  python3 - "${ENV_FILE}" "${key}" "${val}" <<'PY'
import re
import sys

path, key, val = sys.argv[1], sys.argv[2], sys.argv[3]


def fmt_line(k: str, v: str) -> str:
    if re.search(r"[\s#'\"]", v) or v.startswith("-"):
        q = "'" + v.replace("'", "'\"'\"'") + "'"
        return f"{k}={q}\n"
    return f"{k}={v}\n"


with open(path, encoding="utf-8") as f:
    lines = f.readlines()
if lines and not lines[-1].endswith("\n"):
    lines[-1] = lines[-1] + "\n"
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

set -a
# shellcheck disable=SC1090
source "${ENV_FILE}"
set +a

# Linux pool workers: reach claude-tap on the host via host.docker.internal.
POOL_WORKER_RUN_EXTRA="${CLAW_POOL_WORKER_RUN_EXTRA:---add-host host.docker.internal:host-gateway}"
upsert_env_kv "CLAW_DOCKER_EXTRA_ARGS" "${POOL_WORKER_RUN_EXTRA}"
upsert_env_kv "CLAW_PODMAN_EXTRA_ARGS" "${POOL_WORKER_RUN_EXTRA}"

# Worker stdout → POST /v1/internal/turns/{id}/stdout-event (needs both vars in mounted .env).
GATEWAY_HOST_PORT="${GATEWAY_HOST_PORT:-18088}"
if [[ -z "${CLAW_GATEWAY_INTERNAL_BASE_URL:-}" ]]; then
  CLAW_GATEWAY_INTERNAL_BASE_URL="http://claw-gateway-rs:8080"
  upsert_env_kv "CLAW_GATEWAY_INTERNAL_BASE_URL" "${CLAW_GATEWAY_INTERNAL_BASE_URL}"
fi
if [[ -z "${CLAW_PODMAN_NETWORK:-}${CLAW_DOCKER_NETWORK:-}" ]]; then
  upsert_env_kv "CLAW_PODMAN_NETWORK" "stack_default"
fi
if [[ -z "${CLAW_GATEWAY_INTERNAL_TOKEN:-}" ]]; then
  upsert_env_kv "CLAW_GATEWAY_INTERNAL_TOKEN" "claw-internal-dev-token"
fi

echo "OK: pool worker run extras updated (no worker-openai.env snapshot). Set CLAW_WORKER_ENV_FILE in pool daemon to repo .env."

if [[ "${1:-}" == "--restart" ]]; then
  echo "==> restarting tap + gateway (tap-down / down / tap-up / up)"
  "${PODMAN_DIR}/lib/tap-down.sh" || true
  "${PODMAN_DIR}/lib/down.sh" || true
  sleep 0.5
  "${PODMAN_DIR}/lib/tap-up.sh"
  "${PODMAN_DIR}/lib/up.sh"
fi

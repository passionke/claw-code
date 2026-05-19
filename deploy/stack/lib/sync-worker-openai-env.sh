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

echo "OK: pool worker run extras updated (no worker-openai.env snapshot). Set CLAW_WORKER_ENV_FILE in pool daemon to repo .env."

if [[ "${1:-}" == "--restart" ]]; then
  echo "==> restarting stack (stop-with-tap / start-with-tap)"
  "${PODMAN_DIR}/lib/stop-with-tap.sh" || true
  pkill -f 'claude-tap.*--tap-no-launch' 2>/dev/null || true
  sleep 0.5
  # shellcheck source=/dev/null
  source "${PODMAN_DIR}/lib/compose-include.sh"
  RT="$(claw_container_runtime_cli)" || die "need docker or podman"
  mapfile -t STALE < <("${RT}" ps -aq --filter "name=claw-" 2>/dev/null || true)
  if [[ "${#STALE[@]}" -gt 0 ]]; then
    "${RT}" rm -f "${STALE[@]}" || true
  fi
  "${PODMAN_DIR}/lib/start-with-tap.sh"
fi

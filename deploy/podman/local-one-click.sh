#!/usr/bin/env bash
# Local one-click: gateway + worker are always built in this repo (no GHCR pull for claw-code).
#   - deploy/podman/build.sh  ->  claw-gateway-rs:local
#   - Containerfile.gateway-worker  ->  claw-gateway-worker:local
# Host claw-pool-daemon: Linux hosts extract from gateway image; macOS uses cargo-built binary.
# claude-tap: if ../claude-tap exists (Dockerfile or Containerfile), .env is updated to compose-build
# it as claude-tap:local. Override with CLAUDE_TAP_BUILD_CONTEXT / CLAUDE_TAP_IMAGE in .env, or
# LOCAL_ONE_CLICK_USE_GHCR_TAP=1 to keep pulling CLAUDE_TAP_IMAGE from registry.
#
# Uses Podman on macOS when available; otherwise Docker (set CLAW_CONTAINER_RUNTIME=docker in .env).
#
# Prerequisites (repo root):
#   cp .env.example .env
#   Edit .env: OPENAI_API_KEY, UPSTREAM_OPENAI_BASE_URL (real LLM for claude-tap --tap-target).
#
# Usage:
#   ./deploy/podman/local-one-click.sh
#   LOCAL_ONE_CLICK_SKIP_BUILD=1 ./deploy/podman/local-one-click.sh   # only (re)start stack; reuse existing :local images
#
# Author: kejiqing
set -euo pipefail

export LC_ALL="${LC_ALL:-C.UTF-8}"
export LANG="${LANG:-C.UTF-8}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
ENV_FILE="${REPO_ROOT}/.env"

die() {
  echo "error: $*" >&2
  exit 1
}

[[ -f "${ENV_FILE}" ]] || die "missing ${ENV_FILE} — cp .env.example .env and set OPENAI_API_KEY + UPSTREAM_OPENAI_BASE_URL"

if [[ "$(uname -s)" == "Darwin" ]] && command -v podman >/dev/null 2>&1; then
  if ! podman machine list --format '{{.Running}}' 2>/dev/null | grep -q true; then
    echo "==> starting Podman machine (macOS)"
    podman machine start || die "podman machine start failed"
  fi
fi

set -a
# shellcheck disable=SC1090
source "${ENV_FILE}"
set +a

[[ -n "${OPENAI_API_KEY:-}" ]] || die "OPENAI_API_KEY empty in .env"
[[ -n "${UPSTREAM_OPENAI_BASE_URL:-}" ]] || die "UPSTREAM_OPENAI_BASE_URL empty in .env (claude-tap --tap-target)"

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

# shellcheck source=/dev/null
source "${SCRIPT_DIR}/compose-include.sh"
RT="$(claw_container_runtime_cli)" || die "need podman or docker in PATH (see CLAW_CONTAINER_RUNTIME in .env)"

if [[ "${RT}" == "podman" ]]; then
  ISOLATION_DEFAULT="podman_pool"
  POOL_PREFIX="CLAW_PODMAN_"
else
  ISOLATION_DEFAULT="docker_pool"
  POOL_PREFIX="CLAW_DOCKER_"
fi

echo "==> merging local defaults into .env (only keys that are missing; images pinned separately)"
python3 - "${ENV_FILE}" "${ISOLATION_DEFAULT}" "${POOL_PREFIX}" <<'PY'
import re
import sys

path, isolation, pool_pfx = sys.argv[1], sys.argv[2], sys.argv[3]
with open(path, encoding="utf-8") as f:
    text = f.read()


def has_key(k: str) -> bool:
    return re.search(rf"^{re.escape(k)}=", text, re.M) is not None


defaults = [
    ("GATEWAY_HOST_PORT", "8088"),
    ("CLAW_TIMEOUT_SECONDS", "120"),
    ("CLAW_SOLVE_ISOLATION", isolation),
    ("CLAW_CONTAINER_RUNTIME", "podman" if isolation == "podman_pool" else "docker"),
    ("CLAW_POOL_HOST_DAEMON", "1"),
    ("CLAW_POOL_DAEMON_SKIP_BUILD", "1"),
    ("CLAW_HOST_LOG_DIR", "./deploy/podman/claw-logs"),
    ("CLAUDE_TAP_MODE", "docker"),
    ("CLAUDE_TAP_IMAGE", "ghcr.io/passionke/claude-tap:latest"),
    ("CLAUDE_TAP_HOST_PORT", "8080"),
    ("CLAUDE_TAP_LIVE_PORT", "3000"),
    ("CLAUDE_TAP_BIND_HOST", "host.docker.internal"),
]
if isolation == "podman_pool":
    defaults.append(("CLAW_POOL_DAEMON_TCP_HOST", "host.containers.internal"))
else:
    defaults.append(("CLAW_POOL_DAEMON_TCP_HOST", "host.docker.internal"))

defaults += [
    (pool_pfx + "POOL_SIZE", "4"),
    (pool_pfx + "POOL_MIN_IDLE", "1"),
]

missing = [(k, v) for k, v in defaults if not has_key(k)]
if missing:
    with open(path, "a", encoding="utf-8") as f:
        f.write("\n# local-one-click defaults (kejiqing)\n")
        for k, v in missing:
            f.write(f"{k}={v}\n")
PY

echo "==> reload .env after merge"
set -a
# shellcheck disable=SC1090
source "${ENV_FILE}"
set +a

echo "==> pin GATEWAY_IMAGE + worker images to local build (overrides GHCR lines from copied .env)"
upsert_env_kv "GATEWAY_IMAGE" "claw-gateway-rs:local"
upsert_env_kv "CLAW_PODMAN_IMAGE" "claw-gateway-worker:local"
upsert_env_kv "CLAW_DOCKER_IMAGE" "claw-gateway-worker:local"
set -a
# shellcheck disable=SC1090
source "${ENV_FILE}"
set +a

if [[ "${LOCAL_ONE_CLICK_USE_GHCR_TAP:-0}" != "1" ]] && [[ -z "${CLAUDE_TAP_BUILD_CONTEXT:-}" ]]; then
  SIB="${REPO_ROOT}/../claude-tap"
  if [[ -d "${SIB}" ]] && { [[ -f "${SIB}/Dockerfile" ]] || [[ -f "${SIB}/Containerfile" ]]; }; then
    echo "==> sibling claude-tap at ${SIB} → compose build (claude-tap:local); set LOCAL_ONE_CLICK_USE_GHCR_TAP=1 to skip"
    upsert_env_kv "CLAUDE_TAP_BUILD_CONTEXT" "${SIB}"
    upsert_env_kv "CLAUDE_TAP_IMAGE" "claude-tap:local"
    if [[ ! -f "${SIB}/Dockerfile" ]] && [[ -f "${SIB}/Containerfile" ]]; then
      upsert_env_kv "CLAUDE_TAP_DOCKERFILE" "Containerfile"
    fi
    if [[ "$(uname -s)" == "Darwin" ]] && [[ "$(uname -m)" == "arm64" ]] && ! grep -qE '^[[:space:]]*CLAUDE_TAP_PLATFORM=' "${ENV_FILE}"; then
      upsert_env_kv "CLAUDE_TAP_PLATFORM" "linux/arm64"
    fi
    set -a
    # shellcheck disable=SC1090
    source "${ENV_FILE}"
    set +a
  fi
fi

echo "==> refresh tap / OPENAI chain (refresh-tap-llm-chain-in-env.py)"
python3 "${SCRIPT_DIR}/refresh-tap-llm-chain-in-env.py" "${ENV_FILE}"

echo "==> sync worker-openai.env + CLAW_*_EXTRA_ARGS"
"${SCRIPT_DIR}/sync-worker-openai-env.sh"

if [[ "${LOCAL_ONE_CLICK_SKIP_BUILD:-0}" != "1" ]]; then
  echo "==> build gateway image (${RT})"
  "${SCRIPT_DIR}/build.sh" local

  set -a
  # shellcheck disable=SC1090
  source "${ENV_FILE}"
  set +a
  REG="${CONTAINER_BASE_REGISTRY:-docker.1ms.run}"
  REG="${REG%/}"
  RUST_BASE_IMAGE="${REG}/library/rust:1.88-bookworm"
  DEBIAN_BASE_IMAGE="${REG}/library/debian:bookworm-slim"
  echo "==> build worker image claw-gateway-worker:local"
  "${RT}" build \
    --build-arg "RUST_BASE_IMAGE=${RUST_BASE_IMAGE}" \
    --build-arg "DEBIAN_BASE_IMAGE=${DEBIAN_BASE_IMAGE}" \
    -f "${REPO_ROOT}/deploy/podman/Containerfile.gateway-worker" \
    -t claw-gateway-worker:local \
    "${REPO_ROOT}"
else
  echo "==> LOCAL_ONE_CLICK_SKIP_BUILD=1 — skipping image builds"
fi

# Host pool daemon must match the host OS. GATEWAY_IMAGE is Linux — extracting
# /usr/local/bin/claw-pool-daemon only works on Linux hosts; on macOS use cargo. Author: kejiqing
if [[ "$(uname -s)" == "Darwin" ]]; then
  POOL_OUT="${REPO_ROOT}/rust/target/release/claw-pool-daemon"
  echo "==> cargo build host claw-pool-daemon (Darwin; image binary is Linux ELF)"
  (cd "${REPO_ROOT}/rust" && cargo build -p http-gateway-rs --bin claw-pool-daemon --release)
  [[ -x "${POOL_OUT}" ]] || die "missing executable ${POOL_OUT} after cargo build"
else
  POOL_OUT="${CLAW_POOL_DAEMON_BIN:-${HOME}/.local/bin/claw-pool-daemon}"
  if [[ "$(id -u)" -eq 0 ]]; then
    POOL_OUT="${CLAW_POOL_DAEMON_BIN:-/usr/local/bin/claw-pool-daemon}"
  fi
  mkdir -p "$(dirname "${POOL_OUT}")"
  echo "==> install host claw-pool-daemon from GATEWAY_IMAGE -> ${POOL_OUT}"
  export CLAW_POOL_DAEMON_INSTALL_SKIP_PULL=1
  "${SCRIPT_DIR}/install-pool-daemon-from-image.sh" "${POOL_OUT}"
  unset CLAW_POOL_DAEMON_INSTALL_SKIP_PULL
fi
upsert_env_kv "CLAW_POOL_DAEMON_BIN" "${POOL_OUT}"
upsert_env_kv "CLAW_POOL_DAEMON_SKIP_BUILD" "1"

echo "==> stop prior stack + stray claw-* (${RT})"
"${SCRIPT_DIR}/stop-with-tap.sh" || true
sleep 0.5
# macOS ships Bash 3.2 (no mapfile). Author: kejiqing
STALE=()
while IFS= read -r cid; do
  [[ -n "${cid}" ]] && STALE+=("${cid}")
done < <("${RT}" ps -aq --filter "name=claw-" 2>/dev/null || true)
if [[ "${#STALE[@]}" -gt 0 ]]; then
  "${RT}" rm -f "${STALE[@]}" || true
fi

echo "==> start claude-tap + gateway (start-with-tap.sh)"
"${SCRIPT_DIR}/start-with-tap.sh"

set -a
# shellcheck disable=SC1090
source "${ENV_FILE}"
set +a
PORT="${GATEWAY_HOST_PORT:-8088}"
HEALTH_URL="http://127.0.0.1:${PORT}/healthz"

echo "==> health: ${HEALTH_URL}"
BODY=""
for _ in $(seq 1 90); do
  if BODY="$(curl -fsS "${HEALTH_URL}" 2>/dev/null)"; then
    if echo "${BODY}" | python3 -c "import json,sys; j=json.load(sys.stdin); sys.exit(0 if j.get('ok') else 1)" 2>/dev/null; then
      break
    fi
  fi
  BODY=""
  sleep 1
done
[[ -n "${BODY}" ]] || die "healthz never became ready"
echo "${BODY}" | python3 -m json.tool 2>/dev/null || echo "${BODY}"
echo "OK: local stack up. Try: curl -sS ${HEALTH_URL}"

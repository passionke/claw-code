#!/usr/bin/env bash
# Generated worker↔claude-tap wiring (OPENAI_BASE_URL, pool run extras). Called from gateway.sh up/tap-up only.
# Author: kejiqing

# Write deploy/stack/.claw-worker-llm.env and export CLAW_WORKER_ENV_FILE for pool + workers.
# Worker mount must NOT include repo-root `.env` (PG URL, GATEWAY_IMAGE, …). See WORKER_ENV_KEYS in worker_env.rs. kejiqing
claw_ensure_worker_llm_wiring() {
  local script_dir="${1:?}"
  local repo_root llm_env gen bind port openai_tap pool_extra net key val
  repo_root="$(cd "${script_dir}/../.." && pwd)"
  llm_env="${repo_root}/.claw/claw-llm-runtime.env"
  gen="${script_dir}/.claw-worker-llm.env"
  mkdir -p "${repo_root}/.claw"

  bind="${CLAUDE_TAP_BIND_HOST:-host.docker.internal}"
  port="${CLAUDE_TAP_HOST_PORT:-${CLAUDE_TAP_PORT:-8080}}"
  openai_tap="http://${bind}:${port}"
  pool_extra="${CLAW_POOL_WORKER_RUN_EXTRA:---add-host host.docker.internal:host-gateway}"
  net="${CLAW_PODMAN_NETWORK:-${CLAW_DOCKER_NETWORK:-stack_default}}"

  {
    printf '%s\n' '# GENERATED — do not edit. Overwritten by gateway.sh up / tap-up / pool-daemon-up. kejiqing'
    printf '%s\n' '# Worker LLM hits claude-tap; tap upstream from .claw/claw-tap-upstream.json (PG sync).'
    printf '%s\n' "OPENAI_BASE_URL=${openai_tap}"
    printf '%s\n' "INTERNAL_CLAUDE_TAP_HOST=${openai_tap}"
    printf '%s\n' "CLAW_DOCKER_EXTRA_ARGS=${pool_extra}"
    printf '%s\n' "CLAW_PODMAN_EXTRA_ARGS=${pool_extra}"
    printf '%s\n' "CLAW_PODMAN_NETWORK=${net}"
  } >"${gen}"

  # Pool worker mount: PG-synced LLM file + optional deploy tunables (from shell after `source .env`) + tap wiring.
  # No CLAW_GATEWAY_DATABASE_URL / postgres — workers do not talk to PG. kejiqing
  local runtime="${script_dir}/.claw-worker-runtime.env"
  # Subset of WORKER_ENV_KEYS that may be set in human `.env` (keep in sync with worker_env.rs). Author: kejiqing
  local -a deploy_worker_keys=(
    CLAW_MCP_MAX_CONCURRENT
    CLAW_MCP_TOOL_CALL_TIMEOUT_MS
    CLAW_INSTRUCTION_FILE_MAX_CHARS
    CLAW_INSTRUCTION_TOTAL_MAX_CHARS
    CLAW_PROGRESS_MESSAGE_MAX_CHARS
    CLAW_GATEWAY_INTERNAL_BASE_URL
    CLAW_GATEWAY_INTERNAL_TOKEN
  )
  {
    printf '%s\n' '# GENERATED — pool worker mount. Do not edit. kejiqing'
    if [[ -f "${llm_env}" ]]; then
      cat "${llm_env}"
    fi
    for key in "${deploy_worker_keys[@]}"; do
      val="${!key:-}"
      if [[ -n "${val}" ]]; then
        printf '%s\n' "${key}=${val}"
      fi
    done
    cat "${gen}"
  } >"${runtime}"

  export CLAW_WORKER_ENV_FILE="${runtime}"
  export OPENAI_BASE_URL="${openai_tap}"
  export INTERNAL_CLAUDE_TAP_HOST="${openai_tap}"
  export CLAW_DOCKER_EXTRA_ARGS="${pool_extra}"
  export CLAW_PODMAN_EXTRA_ARGS="${pool_extra}"
  export CLAW_PODMAN_NETWORK="${net}"
}

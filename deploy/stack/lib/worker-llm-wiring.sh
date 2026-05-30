#!/usr/bin/env bash
# Pool worker deploy-only env (non-LLM). LLM keys injected per-solve via pool Exec -e. Author: kejiqing

claw_ensure_worker_llm_wiring() {
  local script_dir="${1:?}"
  local repo_root runtime net key val
  repo_root="$(cd "${script_dir}/../.." && pwd)"
  runtime="${script_dir}/.claw-worker-runtime.env"
  net="${CLAW_PODMAN_NETWORK:-${CLAW_DOCKER_NETWORK:-stack_default}}"
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
    printf '%s\n' '# GENERATED — pool worker mount (deploy keys only). LLM via gateway Exec -e. kejiqing'
    for key in "${deploy_worker_keys[@]}"; do
      val="${!key:-}"
      if [[ -n "${val}" ]]; then
        printf '%s\n' "${key}=${val}"
      fi
    done
    printf '%s\n' "CLAW_PODMAN_NETWORK=${net}"
  } >"${runtime}"
  export CLAW_WORKER_ENV_FILE="${runtime}"
  export CLAW_PODMAN_NETWORK="${net}"
}

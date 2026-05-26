#!/usr/bin/env bash
# Generated worker↔claude-tap wiring (OPENAI_BASE_URL, pool run extras). Called from gateway.sh up/tap-up only.
# Author: kejiqing

# Write deploy/stack/.claw-worker-llm.env and export CLAW_WORKER_ENV_FILE for pool + workers.
# Requires repo-root .env already sourced (UPSTREAM_OPENAI_BASE_URL, CLAUDE_TAP_*).
claw_ensure_worker_llm_wiring() {
  local script_dir="${1:?}"
  local repo_root host_env gen bind port openai_tap pool_extra net
  repo_root="$(cd "${script_dir}/../.." && pwd)"
  host_env="${repo_root}/.env"
  gen="${script_dir}/.claw-worker-llm.env"

  bind="${CLAUDE_TAP_BIND_HOST:-host.docker.internal}"
  port="${CLAUDE_TAP_HOST_PORT:-${CLAUDE_TAP_PORT:-8080}}"
  openai_tap="http://${bind}:${port}"
  pool_extra="${CLAW_POOL_WORKER_RUN_EXTRA:---add-host host.docker.internal:host-gateway}"
  net="${CLAW_PODMAN_NETWORK:-${CLAW_DOCKER_NETWORK:-stack_default}}"

  {
    printf '%s\n' '# GENERATED — do not edit. Overwritten by gateway.sh up / tap-up / pool-daemon-up. kejiqing'
    printf '%s\n' '# Worker LLM hits claude-tap; tap forwards to UPSTREAM_OPENAI_BASE_URL in repo .env.'
    printf '%s\n' "OPENAI_BASE_URL=${openai_tap}"
    printf '%s\n' "INTERNAL_CLAUDE_TAP_HOST=${openai_tap}"
    printf '%s\n' "CLAW_DOCKER_EXTRA_ARGS=${pool_extra}"
    printf '%s\n' "CLAW_PODMAN_EXTRA_ARGS=${pool_extra}"
    printf '%s\n' "CLAW_PODMAN_NETWORK=${net}"
  } >"${gen}"

  export CLAW_WORKER_ENV_FILE="${gen}:${host_env}"
  export OPENAI_BASE_URL="${openai_tap}"
  export INTERNAL_CLAUDE_TAP_HOST="${openai_tap}"
  export CLAW_DOCKER_EXTRA_ARGS="${pool_extra}"
  export CLAW_PODMAN_EXTRA_ARGS="${pool_extra}"
  export CLAW_PODMAN_NETWORK="${net}"
}

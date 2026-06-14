# shellcheck shell=bash
# Gateway up: optional LLM + project + claude-tap auto-register from repo .env / CI variables.
# Author: kejiqing

claw_print_gateway_deploy_failure() {
  local ctn="${CLAW_GATEWAY_CONTAINER:-claw-gateway-rs}"
  local rt
  rt="$(command -v docker 2>/dev/null || command -v podman 2>/dev/null || true)"
  echo "==> gateway deploy failure diagnostics" >&2
  if [[ -n "${rt}" ]]; then
    echo "--- ${rt} ps (name=${ctn}) ---" >&2
    "${rt}" ps -a --filter "name=^/${ctn}$" 2>&1 | tail -20 >&2 || true
    echo "--- ${rt} logs ${ctn} (last 120) ---" >&2
    "${rt}" logs --tail 120 "${ctn}" 2>&1 >&2 || true
    echo "--- ${rt} inspect ${ctn} (state) ---" >&2
    "${rt}" inspect "${ctn}" --format 'status={{.State.Status}} exit={{.State.ExitCode}} err={{.State.Error}} oom={{.State.OOMKilled}}' 2>&1 >&2 || true
  fi
}

claw_wait_gateway_http_ready() {
  local max_attempts="${1:-60}"
  local port="${GATEWAY_HOST_PORT:-18088}"
  local i
  for i in $(seq 1 "${max_attempts}"); do
    if curl -fsS --connect-timeout 2 "http://127.0.0.1:${port}/healthz" >/dev/null 2>&1; then
      echo "gateway HTTP ready (attempt ${i}/${max_attempts})"
      return 0
    fi
    echo "waiting gateway HTTP (${i}/${max_attempts})…" >&2
    sleep 2
  done
  echo "error: gateway HTTP not ready on :${port}" >&2
  claw_print_gateway_deploy_failure
  return 1
}

claw_bootstrap_llm_api_key() {
  printf '%s' "${CLAW_BOOTSTRAP_LLM_API_KEY:-${OPENAI_API_KEY:-}}"
}

claw_bootstrap_llm_base_url() {
  printf '%s' "${CLAW_BOOTSTRAP_LLM_BASE_URL:-${UPSTREAM_OPENAI_BASE_URL:-${OPENAI_BASE_URL:-}}}"
}

claw_bootstrap_llm_model_name() {
  printf '%s' "${CLAW_BOOTSTRAP_LLM_MODEL_NAME:-${OPENAI_MODEL:-gpt-4o-mini}}"
}

claw_bootstrap_llm_from_env() {
  local port="${GATEWAY_HOST_PORT:-18088}"
  local key base model name body force
  force="${CLAW_BOOTSTRAP_LLM_FORCE:-0}"
  if [[ "${force}" != "1" ]] && claw_gateway_has_active_llm; then
    echo "bootstrap: active LLM already in PG"
    return 0
  fi
  key="$(claw_bootstrap_llm_api_key)"
  base="$(claw_bootstrap_llm_base_url)"
  model="$(claw_bootstrap_llm_model_name)"
  name="${CLAW_BOOTSTRAP_LLM_NAME:-ci-bootstrap}"
  if [[ -z "${key}" || -z "${base}" ]]; then
    return 1
  fi
  body="$(python3 -c 'import json,sys; print(json.dumps({
    "name": sys.argv[1],
    "baseModelUrl": sys.argv[2],
    "modelName": sys.argv[3],
    "apiKey": sys.argv[4],
  }, ensure_ascii=False))' "${name}" "${base}" "${model}" "${key}")"
  echo "==> bootstrap PUT /v1/gateway/global-settings/active-llm-config (${name})" >&2
  local resp http_code
  resp="$(mktemp)"
  http_code="$(curl -sS --connect-timeout 15 -o "${resp}" -w '%{http_code}' -X PUT \
    "http://127.0.0.1:${port}/v1/gateway/global-settings/active-llm-config" \
    -H 'Content-Type: application/json' \
    -d "${body}")" || http_code="000"
  if [[ "${http_code}" != "200" ]]; then
    echo "error: bootstrap LLM PUT HTTP ${http_code}: $(tr -d '\n' <"${resp}" | head -c 500)" >&2
    rm -f "${resp}"
    return 1
  fi
  rm -f "${resp}"
  if ! claw_gateway_has_active_llm; then
    echo "error: bootstrap LLM apply did not set activeLlmConfig" >&2
    return 1
  fi
  echo "bootstrap: active LLM configured (model=${model})"
  return 0
}

# Default project (projId=1): POST /v1/projects + /v1/init; verify GET config 200. Author: kejiqing
claw_ensure_default_project_ds() {
  local proj_id="${1:-${CLAW_BOOTSTRAP_PROJ_ID:-${CLAW_BOOTSTRAP_DS_ID:-1}}}"
  local port="${GATEWAY_HOST_PORT:-18088}"
  local code resp http_code

  code="$(curl -sS -o /dev/null -w '%{http_code}' --connect-timeout 15 \
    "http://127.0.0.1:${port}/v1/project/config/${proj_id}" 2>/dev/null || echo 000)"
  if [[ "${code}" != "200" ]]; then
    echo "==> bootstrap POST /v1/projects projId=${proj_id} (config was HTTP ${code})" >&2
    resp="$(mktemp)"
    http_code="$(curl -sS --connect-timeout 120 -o "${resp}" -w '%{http_code}' -X POST \
      "http://127.0.0.1:${port}/v1/projects" \
      -H 'Content-Type: application/json' \
      -d "{\"projId\":${proj_id}}" 2>/dev/null || echo 000)"
    if [[ "${http_code}" != "200" && "${http_code}" != "409" ]]; then
      echo "error: POST /v1/projects proj=${proj_id} HTTP ${http_code}: $(tr -d '\n' <"${resp}" | head -c 500)" >&2
      rm -f "${resp}"
      return 1
    fi
    rm -f "${resp}"
    echo "bootstrap: POST /v1/projects proj=${proj_id} HTTP ${http_code}"
  else
    echo "bootstrap: project_config proj=${proj_id} exists (GET 200)"
  fi

  echo "==> bootstrap POST /v1/init projId=${proj_id}" >&2
  resp="$(mktemp)"
  http_code="$(curl -sS --connect-timeout 120 -o "${resp}" -w '%{http_code}' -X POST \
    "http://127.0.0.1:${port}/v1/init" \
    -H 'Content-Type: application/json' \
    -d "{\"projId\":${proj_id}}" 2>/dev/null || echo 000)"
  if [[ "${http_code}" != "200" ]]; then
    echo "error: POST /v1/init proj=${proj_id} HTTP ${http_code}: $(tr -d '\n' <"${resp}" | head -c 500)" >&2
    rm -f "${resp}"
    return 1
  fi
  rm -f "${resp}"

  code="$(curl -sS -o /dev/null -w '%{http_code}' --connect-timeout 15 \
    "http://127.0.0.1:${port}/v1/project/config/${proj_id}" 2>/dev/null || echo 000)"
  if [[ "${code}" != "200" ]]; then
    echo "error: proj=${proj_id} still missing after bootstrap (GET config HTTP ${code})" >&2
    return 1
  fi
  echo "bootstrap: proj=${proj_id} registered (project + init, GET config 200)"
}

claw_bootstrap_project_if_missing() {
  claw_ensure_default_project_ds "$@"
}

claw_claude_tap_up_and_register() {
  local podman_dir="$1"
  local root_dir="$2"
  # shellcheck source=/dev/null
  source "${podman_dir}/lib/claude-tap-local.sh"
  claw_claude_tap_start "${podman_dir}" "${root_dir}"
  claw_ensure_worker_llm_wiring "${podman_dir}"
  if claw_claude_tap_register_in_admin; then
    claw_wait_gateway_claw_tap_ready 30
  else
    echo "error: clawTap register in Admin failed" >&2
    return 1
  fi
}

# LLM (env) → default project → tap-up + Admin clawTap register. Called from gateway.sh up.
claw_bootstrap_gateway_runtime() {
  local podman_dir="$1"
  local root_dir="$2"
  local auto="${CLAW_AUTO_BOOTSTRAP:-0}"

  claw_wait_gateway_http_ready 45

  if ! claw_gateway_has_active_llm || [[ "${CLAW_BOOTSTRAP_LLM_FORCE:-0}" == "1" ]]; then
    if claw_bootstrap_llm_from_env; then
      :
    elif [[ "${auto}" == "1" ]]; then
      echo "error: CLAW_AUTO_BOOTSTRAP=1 but no CLAW_BOOTSTRAP_LLM_API_KEY/OPENAI_API_KEY + base URL in .env" >&2
      return 1
    else
      echo "note: skip claude-tap — no active LLM in PG (cluster=${CLAW_CLUSTER_ID:-unset})" >&2
      echo "      set CLAW_BOOTSTRAP_LLM_* or OPENAI_API_KEY + UPSTREAM_OPENAI_BASE_URL in .env" >&2
      echo "      or Admin :${GATEWAY_PLAYGROUND_HOST_PORT:-18765}/admin → 全局推理 Apply" >&2
      return 0
    fi
  fi

  echo "==> claude-tap up + Admin register (CLAUDE_TAP_MODE=${CLAUDE_TAP_MODE:-docker})" >&2
  claw_claude_tap_up_and_register "${podman_dir}" "${root_dir}"
}

# shellcheck shell=bash
# Gateway up: optional LLM + project + claude-tap auto-register from repo .env / CI variables.
# Author: kejiqing

claw_wait_gateway_http_ready() {
  local max_attempts="${1:-45}"
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
  local key base model name body
  if claw_gateway_has_active_llm; then
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
  body="$(python3 -c 'import json,os; print(json.dumps({
    "name": os.environ["NAME"],
    "baseModelUrl": os.environ["BASE"],
    "modelName": os.environ["MODEL"],
    "apiKey": os.environ["KEY"],
  }, ensure_ascii=False))' \
    NAME="${name}" BASE="${base}" MODEL="${model}" KEY="${key}")"
  echo "==> bootstrap PUT /v1/gateway/global-settings/active-llm-config (${name})" >&2
  curl -fsS --connect-timeout 15 -X PUT \
    "http://127.0.0.1:${port}/v1/gateway/global-settings/active-llm-config" \
    -H 'Content-Type: application/json' \
    -d "${body}" >/dev/null
  if ! claw_gateway_has_active_llm; then
    echo "error: bootstrap LLM apply did not set activeLlmConfig" >&2
    return 1
  fi
  echo "bootstrap: active LLM configured (model=${model})"
  return 0
}

claw_bootstrap_project_if_missing() {
  local ds_id="${1:-1}"
  local port="${GATEWAY_HOST_PORT:-18088}"
  local code
  code="$(curl -sS -o /dev/null -w '%{http_code}' --connect-timeout 10 \
    "http://127.0.0.1:${port}/v1/project/config/${ds_id}" 2>/dev/null || echo 000)"
  if [[ "${code}" == "200" ]]; then
    echo "bootstrap: project_config ds=${ds_id} exists"
    return 0
  fi
  echo "==> bootstrap POST /v1/projects dsId=${ds_id}" >&2
  curl -fsS --connect-timeout 60 -X POST "http://127.0.0.1:${port}/v1/projects" \
    -H 'Content-Type: application/json' \
    -d "{\"dsId\":${ds_id}}"
  echo
  echo "bootstrap: project ds=${ds_id} created"
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

  if ! claw_gateway_has_active_llm; then
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

  claw_bootstrap_project_if_missing "${CLAW_BOOTSTRAP_DS_ID:-1}" || {
    [[ "${auto}" == "1" ]] && return 1
    echo "note: project bootstrap skipped or failed (non-fatal)" >&2
  }

  echo "==> claude-tap up + Admin register (CLAUDE_TAP_MODE=${CLAUDE_TAP_MODE:-docker})" >&2
  claw_claude_tap_up_and_register "${podman_dir}" "${root_dir}"
}

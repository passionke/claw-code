#!/usr/bin/env bash
# CI cluster gate: per-gateway workspace + node B solve + cross-gateway session. Author: kejiqing
set -euo pipefail

LIB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PODMAN_DIR="$(cd "${LIB_DIR}/.." && pwd)"
REPO_ROOT="$(cd "${PODMAN_DIR}/../.." && pwd)"
ENV_A="${REPO_ROOT}/.env"
ENV_B="${REPO_ROOT}/.env.ci-node-b"

fail() {
  echo "CI CLUSTER SOLVE E2E FAIL: $*" >&2
  exit 1
}

ok() {
  echo "CI CLUSTER SOLVE E2E OK: $*"
}

[[ -f "${ENV_A}" ]] || fail "missing ${ENV_A}"
[[ -f "${ENV_B}" ]] || fail "missing ${ENV_B} — run ci-cluster-dual-deploy first"

set -a
# shellcheck disable=SC1090
source "${ENV_A}"
set +a
GW_A="${GATEWAY_HOST_PORT:-18088}"
POOL_A="${CLAW_POOL_ID:-pool-sunmi-ci-01}"
PROJ_ID="${CLAW_BOOTSTRAP_PROJ_ID:-${CLAW_BOOTSTRAP_DS_ID:-1}}"
WS_A="${PODMAN_DIR}/claw-workspace"

set -a
# shellcheck disable=SC1090
source "${ENV_B}"
set +a
GW_B="${GATEWAY_HOST_PORT:-18089}"
POOL_B="${CLAW_POOL_ID:-pool-sunmi-ci-02}"
POOL_HTTP_B="${CLAW_POOL_HTTP_PORT:-9964}"
WS_B="${PODMAN_DIR}/claw-workspace-ci-b"

# shellcheck disable=SC1091
source "${LIB_DIR}/compose-include.sh"
# shellcheck disable=SC1091
source "${LIB_DIR}/e2e-project-isolation.sh"

claw_ci_workspace_writable() {
  local ws="$1" label="$2"
  local rt="${3:?}" uid="${CLAW_WORKER_UID:-1000}" gid="${CLAW_WORKER_GID:-1000}"
  local img="${CLAW_CHOWN_RUNNER_IMAGE:-docker.1ms.run/library/alpine:3.20}"
  [[ -d "${ws}" ]] || fail "${label} workspace missing: ${ws}"
  "${rt}" run --rm -u "${uid}:${gid}" -v "${ws}:/w:rw" "${img}" \
    sh -c 'touch /w/.ci-cluster-write-probe && rm -f /w/.ci-cluster-write-probe' \
    || fail "${label} workspace not writable as uid ${uid}: ${ws}"
}

claw_ci_cluster_solve_e2e_run() {
  local env_file="$1" gw_port="$2" pool_id="$3" isolation="${4:-}" expect_iso="${5:-}" session_id="${6:-}"
  (
    set -a
    # shellcheck disable=SC1090
    source "${env_file}"
    set +a
    export GATEWAY_HOST_PORT="${gw_port}"
    export CLAW_E2E_EXPECT_POOL_ID="${pool_id}"
    if [[ -n "${expect_iso}" ]]; then
      export CLAW_E2E_EXPECT_WORKER_ISOLATION="${expect_iso}"
    else
      unset CLAW_E2E_EXPECT_WORKER_ISOLATION || true
    fi
    if [[ -n "${isolation}" ]]; then
      export CLAW_E2E_WORKER_ISOLATION="${isolation}"
    else
      unset CLAW_E2E_WORKER_ISOLATION || true
    fi
    if [[ -n "${session_id}" ]]; then
      export CLAW_E2E_SESSION_ID="${session_id}"
    else
      unset CLAW_E2E_SESSION_ID || true
    fi
    "${LIB_DIR}/admin-solve-e2e.sh" "${PROJ_ID}" ping
  )
}

claw_ci_solve_capture_session() {
  local env_file="$1" gw_port="$2" out sid
  out="$(mktemp)"
  (
    set -a
    # shellcheck disable=SC1090
    source "${env_file}"
    set +a
    export GATEWAY_HOST_PORT="${gw_port}"
    export CLAW_E2E_SESSION_OUT_FILE="${out}"
    unset CLAW_E2E_SESSION_ID CLAW_E2E_EXPECT_POOL_ID CLAW_E2E_EXPECT_WORKER_ISOLATION CLAW_E2E_WORKER_ISOLATION || true
    "${LIB_DIR}/admin-solve-e2e.sh" "${PROJ_ID}" ping
  ) >&2
  sid="$(tr -d '\r\n' <"${out}" 2>/dev/null || true)"
  rm -f "${out}"
  printf '%s' "${sid}"
}

RT="$(claw_container_runtime_cli 2>/dev/null || true)"
[[ -n "${RT}" ]] || fail "need docker or podman"

echo "==> [1/7] per-gateway workspace (A=${WS_A} B=${WS_B})"
[[ "${WS_A}" != "${WS_B}" ]] || fail "node A/B must use separate workspace binds (got same ${WS_A})"

echo "==> [2/7] each workspace writable as gateway uid ${CLAW_WORKER_UID:-1000}"
claw_ci_workspace_writable "${WS_A}" "node A" "${RT}"
claw_ci_workspace_writable "${WS_B}" "node B" "${RT}"

echo "==> [3/7] node B pool HTTP :${POOL_HTTP_B}"
curl -fsS --connect-timeout 5 "http://127.0.0.1:${POOL_HTTP_B}/healthz/live-report" >/dev/null \
  || fail "node B pool not reachable on :${POOL_HTTP_B}"

echo "==> [4/7] node B gateway solve strict ×2 (:${GW_B} pool=${POOL_B})"
claw_ci_cluster_solve_e2e_run "${ENV_B}" "${GW_B}" "${POOL_B}" "" strict
claw_ci_cluster_solve_e2e_run "${ENV_B}" "${GW_B}" "${POOL_B}" "" strict

echo "==> [5/7] cross-gateway session: created on A, first turn on B (local dir recreate)"
SESSION_ID="$(claw_ci_solve_capture_session "${ENV_A}" "${GW_A}")"
[[ "${SESSION_ID}" =~ ^[0-9a-f]{32}$ ]] || fail "invalid sessionId from node A solve: ${SESSION_ID:-<empty>}"
echo "    sessionId=${SESSION_ID}"
claw_ci_cluster_solve_e2e_run "${ENV_B}" "${GW_B}" "${POOL_B}" "" strict "${SESSION_ID}"

echo "==> [6/7] node B gateway solve relaxed (:${GW_B} pool=${POOL_B})"
claw_ci_cluster_solve_e2e_run "${ENV_B}" "${GW_B}" "${POOL_B}" relaxed relaxed

echo "==> [7/7] node A still solves after cluster (:${GW_A} pool=${POOL_A})"
claw_e2e_set_project_worker_isolation "${GW_A}" "${PROJ_ID}" strict
claw_ci_cluster_solve_e2e_run "${ENV_A}" "${GW_A}" "${POOL_A}" "" strict

ok "per-gateway workspace + node B solve + cross-gateway session passed"

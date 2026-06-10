#!/usr/bin/env bash
# CI cluster gate: node B solve (strict + relaxed) + shared workspace uid 1000. Author: kejiqing
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

set -a
# shellcheck disable=SC1090
source "${ENV_B}"
set +a
GW_B="${GATEWAY_HOST_PORT:-18089}"
POOL_B="${CLAW_POOL_ID:-pool-sunmi-ci-02}"
POOL_HTTP_B="${CLAW_POOL_HTTP_PORT:-9964}"

# shellcheck disable=SC1091
source "${LIB_DIR}/compose-include.sh"
# shellcheck disable=SC1091
source "${LIB_DIR}/e2e-project-isolation.sh"

claw_ci_cluster_solve_e2e_run() {
  local env_file="$1" gw_port="$2" pool_id="$3" isolation="${4:-}" expect_iso="${5:-}"
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
    "${LIB_DIR}/admin-solve-e2e.sh" "${PROJ_ID}" ping
  )
}

claw_podman_export_pool_workspace "${PODMAN_DIR}"
WS_A="$(claw_stack_workspace_bind_dir "${PODMAN_DIR}")"
WS_B="${CLAW_POOL_WORK_ROOT_BIND_SRC:?}"

RT="$(claw_container_runtime_cli 2>/dev/null || true)"
[[ -n "${RT}" ]] || fail "need docker or podman"

echo "==> [1/6] shared workspace: node A/B bind same path (${WS_A})"
[[ "${WS_A}" == "${WS_B}" ]] || fail "node B workspace ${WS_B} != node A ${WS_A} (shared PG requires one bind)"

echo "==> [2/6] workspace writable as gateway uid ${CLAW_WORKER_UID:-1000}"
CHOWN_IMG="${CLAW_CHOWN_RUNNER_IMAGE:-docker.1ms.run/library/alpine:3.20}"
"${RT}" run --rm -u "${CLAW_WORKER_UID:-1000}:${CLAW_WORKER_GID:-1000}" -v "${WS_A}:/w:rw" "${CHOWN_IMG}" \
  sh -c 'touch /w/.ci-cluster-write-probe && rm -f /w/.ci-cluster-write-probe' \
  || fail "uid ${CLAW_WORKER_UID:-1000} cannot write ${WS_A} (Permission denied repro)"

echo "==> [3/6] node B pool HTTP :${POOL_HTTP_B}"
curl -fsS --connect-timeout 5 "http://127.0.0.1:${POOL_HTTP_B}/healthz/live-report" >/dev/null \
  || fail "node B pool not reachable on :${POOL_HTTP_B}"

echo "==> [4/6] node B gateway solve strict ×2 (:${GW_B} pool=${POOL_B})"
claw_ci_cluster_solve_e2e_run "${ENV_B}" "${GW_B}" "${POOL_B}" "" strict
claw_ci_cluster_solve_e2e_run "${ENV_B}" "${GW_B}" "${POOL_B}" "" strict

echo "==> [5/6] node B gateway solve relaxed (:${GW_B} pool=${POOL_B})"
claw_ci_cluster_solve_e2e_run "${ENV_B}" "${GW_B}" "${POOL_B}" relaxed relaxed

echo "==> [6/6] node A still solves after cluster (:${GW_A} pool=${POOL_A})"
claw_e2e_set_project_worker_isolation "${GW_A}" "${PROJ_ID}" strict
claw_ci_cluster_solve_e2e_run "${ENV_A}" "${GW_A}" "${POOL_A}" "" strict

ok "node B strict×2 + relaxed + node A regression passed"

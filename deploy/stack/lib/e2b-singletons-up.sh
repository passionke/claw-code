#!/usr/bin/env bash
# Ensure e2b singletons (nas-api / ovs / observe) and persist endpoints to PG. Author: kejiqing
set -euo pipefail

LIB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PODMAN_DIR="$(cd "${LIB_DIR}/.." && pwd)"
REPO_ROOT="$(cd "${PODMAN_DIR}/../.." && pwd)"
ENV_FILE="${REPO_ROOT}/.env"
# shellcheck source=stack-instance.sh
source "${LIB_DIR}/stack-instance.sh"

if [[ ! -f "${ENV_FILE}" ]]; then
  echo "error: missing ${ENV_FILE}" >&2
  exit 1
fi

set -a
# shellcheck disable=SC1090
source "${ENV_FILE}"
set +a

reuse=0
reset=0
for arg in "$@"; do
  case "${arg}" in
    --reuse) reuse=1 ;;
    --reset) reset=1 ;;
  esac
done

args=()
[[ "${reuse}" -eq 1 ]] && args+=(--reuse)
[[ "${reset}" -eq 1 ]] && args+=(--reset)

echo "==> e2b singletons (PG $(claw_redact_database_url "${CLAW_GATEWAY_DATABASE_URL}"))" >&2
echo "    API ${CLAW_E2B_API_URL:-unset}" >&2

"${LIB_DIR}/e2b-nas-api-up.sh" "${args[@]}"
"${LIB_DIR}/e2b-ovs-up.sh" "${args[@]}"
"${LIB_DIR}/e2b-tap-live-up.sh" "${args[@]}"

echo "OK — nas-api + ovs + observe singletons ready in PG" >&2
echo "next: ./deploy/stack/gateway.sh up --release release-vX.Y.Z" >&2

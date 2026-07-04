#!/usr/bin/env bash
# Ensure e2b singletons via gateway admin API (nas-api / ovs / observe). Author: kejiqing
set -euo pipefail

LIB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PODMAN_DIR="$(cd "${LIB_DIR}/.." && pwd)"
REPO_ROOT="$(cd "${PODMAN_DIR}/../.." && pwd)"
ENV_FILE="${REPO_ROOT}/.env"
# shellcheck source=stack-instance.sh
source "${LIB_DIR}/stack-instance.sh"
# shellcheck source=bootstrap-runtime.sh
source "${LIB_DIR}/bootstrap-runtime.sh"

if [[ ! -f "${ENV_FILE}" ]]; then
  echo "error: missing ${ENV_FILE}" >&2
  exit 1
fi

set -a
# shellcheck disable=SC1090
source "${ENV_FILE}"
set +a

reset=0
for arg in "$@"; do
  case "${arg}" in
    --reset) reset=1 ;;
    --reuse) ;; # legacy alias: ensure only
  esac
done

gw_port="${GATEWAY_HOST_PORT:-18088}"
gw_base="http://127.0.0.1:${gw_port}"

echo "==> e2b singletons via gateway API (${gw_base})" >&2
echo "    PG $(claw_redact_database_url "${CLAW_GATEWAY_DATABASE_URL}")" >&2

if ! claw_wait_gateway_http_ready 30; then
  echo "error: gateway not reachable at ${gw_base}" >&2
  echo "hint: run ./deploy/stack/gateway.sh up first (singletons auto-ensure on startup)" >&2
  exit 1
fi

action="ensure"
if [[ "${reset}" -eq 1 ]]; then
  action="reset"
fi

for component in nas-api ovs observe; do
  echo "    ${action} ${component} ..." >&2
  curl -fsS -X POST "${gw_base}/v1/gateway/global-settings/e2b-singletons/${component}/${action}" \
    -H "Content-Type: application/json" \
    -d '{}' >/dev/null
done

echo "OK — nas-api + ovs + observe singletons ensured via gateway API" >&2

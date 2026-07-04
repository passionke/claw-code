#!/usr/bin/env bash
# Ensure e2b OVS singleton via gateway API. Author: kejiqing
set -euo pipefail

LIB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=bootstrap-runtime.sh
source "${LIB_DIR}/bootstrap-runtime.sh"

reset=0
for arg in "$@"; do
  case "${arg}" in
    --reset) reset=1 ;;
  esac
done

gw_port="${GATEWAY_HOST_PORT:-18088}"
gw_base="http://127.0.0.1:${gw_port}"
action="ensure"
[[ "${reset}" -eq 1 ]] && action="reset"

claw_wait_gateway_http_ready 30 || {
  echo "error: gateway not reachable at ${gw_base}" >&2
  exit 1
}

curl -fsS -X POST "${gw_base}/v1/gateway/global-settings/e2b-singletons/ovs/${action}" \
  -H "Content-Type: application/json" \
  -d '{}'
echo

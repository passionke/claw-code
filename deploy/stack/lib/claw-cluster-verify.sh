#!/usr/bin/env bash
# Shared-PG multi-host gate: claw_pool hygiene + each gateway /healthz + /v1/pools. Author: kejiqing
# Run from ANY cluster node after all hosts upgraded (pre-prod / prod). Not a substitute for per-host verify.
set -euo pipefail

LIB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PODMAN_DIR="$(cd "${LIB_DIR}/.." && pwd)"
REPO_ROOT="$(cd "${PODMAN_DIR}/../.." && pwd)"
ENV_FILE="${REPO_ROOT}/.env"

fail() {
  echo "CLUSTER VERIFY FAIL: $*" >&2
  exit 1
}

ok() {
  echo "CLUSTER VERIFY OK: $*"
}

if [[ ! -f "${ENV_FILE}" ]]; then
  fail "missing ${ENV_FILE}"
fi
set -a
# shellcheck disable=SC1090
source "${ENV_FILE}"
set +a

# shellcheck source=compose-include.sh
source "${PODMAN_DIR}/lib/compose-include.sh"
# shellcheck source=pool-health.sh
source "${LIB_DIR}/pool-health.sh"

RT="$(claw_container_runtime_cli 2>/dev/null || true)"
[[ -n "${RT}" ]] || fail "need docker or podman in PATH"

PG_USER="${CLAW_GATEWAY_PG_USER:-claw_gateway}"
PG_DB="${CLAW_GATEWAY_PG_DATABASE:-claw_gateway}"
PG_CTN="${CLAW_GATEWAY_PG_CONTAINER:-claw-gateway-postgres}"
PG_PORT="${CLAW_GATEWAY_PG_HOST_PORT:-5433}"

psql_q() {
  if claw_compose_uses_local_postgres; then
    "${RT}" exec "${PG_CTN}" psql -U "${PG_USER}" -d "${PG_DB}" -t -A -c "$1"
    return 0
  fi
  local url pg_img
  url="$(claw_pool_daemon_database_url)" || fail "CLAW_GATEWAY_DATABASE_URL unset"
  pg_img="${CLAW_GATEWAY_PG_IMAGE:-docker.io/library/postgres:17-alpine}"
  "${RT}" run --rm "${pg_img}" psql "${url}" -t -A -c "$1"
}

echo "==> [1/4] claw_pool: no legacy dual-pool rows (*-strict / *-relaxed)"

legacy="$(psql_q "
  SELECT pool_id
  FROM claw_pool
  WHERE pool_id LIKE '%-strict' OR pool_id LIKE '%-relaxed'
  ORDER BY pool_id;
" | sed '/^$/d')"

if [[ -n "${legacy}" ]]; then
  online_legacy="$(psql_q "
    SELECT pool_id
    FROM claw_pool
    WHERE (pool_id LIKE '%-strict' OR pool_id LIKE '%-relaxed')
      AND (EXTRACT(EPOCH FROM NOW())*1000 - last_heartbeat_ms) < 120000
    ORDER BY pool_id;
  " | sed '/^$/d')"
  if [[ -n "${online_legacy}" ]]; then
    echo "online legacy dual-pool rows (stop old daemons, then DELETE /v1/pools/{id}):" >&2
    printf '%s\n' "${online_legacy}" >&2
    fail "online *-strict / *-relaxed pool_id rows remain (dual-pool daemons still running?)"
  fi
  echo "pruning offline legacy dual-pool rows:" >&2
  printf '%s\n' "${legacy}" >&2
  psql_q "DELETE FROM claw_pool WHERE pool_id LIKE '%-strict' OR pool_id LIKE '%-relaxed';" >/dev/null || true
  ok "pruned offline legacy dual-pool pool_id rows"
else
  ok "no legacy dual-pool pool_id rows"
fi

echo "==> [2/4] each gateway_base has online pool row"

bad_gw="$(psql_q "
  SELECT DISTINCT gateway_base
  FROM claw_pool
  WHERE gateway_base <> ''
    AND gateway_base NOT IN (
      SELECT gateway_base FROM claw_pool
      WHERE gateway_base <> ''
        AND (EXTRACT(EPOCH FROM NOW())*1000 - last_heartbeat_ms) < 120000
    )
  ORDER BY 1;
" | sed '/^$/d')"

if [[ -n "${bad_gw}" ]]; then
  echo "gateway_base without online pool row:" >&2
  printf '%s\n' "${bad_gw}" >&2
  fail "every cluster gateway must have online pool row in claw_pool"
fi
ok "each registered gateway has online pool row"

echo "==> [3/4] probe gateways (healthz + /v1/pools coLocatedPoolId)"

gateways="$(psql_q "
  SELECT DISTINCT gateway_base
  FROM claw_pool
  WHERE gateway_base <> ''
    AND (EXTRACT(EPOCH FROM NOW())*1000 - last_heartbeat_ms) < 120000
  ORDER BY 1;
" | sed '/^$/d')"

[[ -n "${gateways}" ]] || fail "no online pools in claw_pool — cluster empty?"

while IFS= read -r gw; do
  [[ -n "${gw}" ]] || continue
  gw="${gw%/}"
  echo "    probe ${gw}"
  hz="$(curl -fsS --connect-timeout 5 "${gw}/healthz")" || fail "GET ${gw}/healthz failed"
  tag="$(printf '%s' "${hz}" | python3 -c 'import json,sys; print(json.load(sys.stdin).get("deployImageTag") or "")' 2>/dev/null || true)"
  pools_json="$(curl -fsS --connect-timeout 5 "${gw}/v1/pools")" || fail "GET ${gw}/v1/pools failed"
  python3 - "${pools_json}" "${gw}" "${tag}" <<'PY'
import json, sys
body, gw, tag = sys.argv[1], sys.argv[2], sys.argv[3]
data = json.loads(body)
co = (data.get("coLocatedPoolId") or "").strip()
if not co:
    raise SystemExit(f"{gw}: coLocatedPoolId empty")
online = [p for p in data.get("pools") or [] if p.get("online")]
co_online = [p for p in online if str(p.get("poolId", "")) == co]
if not co_online:
    raise SystemExit(f"{gw}: coLocatedPoolId={co!r} not online in /v1/pools")
print(f"      deployImageTag={tag or '?'} coLocated={co} online_pools={len(online)}")
PY
done <<<"${gateways}"

ok "all cluster gateways respond; coLocatedPoolId online"

echo "==> [4/4] offline row count (informational)"
offline_n="$(psql_q "
  SELECT count(*)::text FROM claw_pool
  WHERE (EXTRACT(EPOCH FROM NOW())*1000 - last_heartbeat_ms) >= 120000;
")"
total_n="$(psql_q "SELECT count(*)::text FROM claw_pool;")"
echo "    claw_pool rows: total=${total_n} offline=${offline_n}"
if [[ "${offline_n}" != "0" ]]; then
  echo "    hint: delete offline zombies in Admin → Pool 集群 (daemon re-registers on pool-up)" >&2
fi

ok "cluster registry verify complete"

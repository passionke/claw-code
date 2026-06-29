#!/usr/bin/env bash
# Standalone PostgreSQL on infra host (10.8.0.1). Independent of gateway stack. Author: kejiqing
set -euo pipefail

LIB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PODMAN_DIR="$(cd "${LIB_DIR}/.." && pwd)"
REPO_ROOT="$(cd "${PODMAN_DIR}/../.." && pwd)"
ENV_FILE="${CLAW_INFRA_PG_ENV_FILE:-${REPO_ROOT}/.env}"

if [[ ! -f "${ENV_FILE}" ]]; then
  echo "error: missing ${ENV_FILE}" >&2
  echo "hint: cp .env.example ${ENV_FILE} && set CLAW_GATEWAY_DATABASE_URL=postgres://...@10.8.0.1:5433/..." >&2
  exit 1
fi

set -a
# shellcheck disable=SC1090
source "${ENV_FILE}"
set +a

# shellcheck disable=SC1091
source "${LIB_DIR}/compose-include.sh"

RT="$(claw_container_runtime_cli)" || exit 1
PROJECT="${CLAW_INFRA_PG_COMPOSE_PROJECT:-claw-infra-pg}"
PG_CTN="${CLAW_COMPOSE_PG_CONTAINER:-claw-infra-postgres}"
PG_PORT="${CLAW_GATEWAY_PG_HOST_PORT:-5433}"
PG_ADVERTISE_HOST="${CLAW_INFRA_PG_ADVERTISE_HOST:-10.8.0.1}"
PG_DATA="${CLAW_PG_DATA_DIR:-${PODMAN_DIR}/claw-pg-data-infra}"

export CLAW_PG_DATA_DIR="${PG_DATA}"
export CLAW_COMPOSE_PG_CONTAINER="${PG_CTN}"
export CLAW_GATEWAY_PG_HOST_PORT="${PG_PORT}"

echo "==> infra PG on 0.0.0.0:${PG_PORT} (container ${PG_CTN}, data ${PG_DATA})" >&2

"${RT}" compose -p "${PROJECT}" -f "${PODMAN_DIR}/podman-compose.postgres-only.yml" \
  --env-file "${ENV_FILE}" up -d postgres

for i in $(seq 1 30); do
  if "${RT}" exec "${PG_CTN}" pg_isready -U "${CLAW_GATEWAY_PG_USER:-claw_gateway}" \
    -d "${CLAW_GATEWAY_PG_DATABASE:-claw_gateway}" >/dev/null 2>&1; then
    APP_USER="${CLAW_GATEWAY_DB_APP_USER:-claw_gateway_app}"
    APP_PASSWORD="${CLAW_GATEWAY_DB_APP_PASSWORD:-${CLAW_GATEWAY_PG_PASSWORD:-clawGw9Dev_Pg}}"
    "${RT}" exec -i "${PG_CTN}" psql -U "${CLAW_GATEWAY_PG_USER:-claw_gateway}" \
      -d "${CLAW_GATEWAY_PG_DATABASE:-claw_gateway}" \
      -v ON_ERROR_STOP=1 \
      -v app_user="${APP_USER}" \
      -v app_password="${APP_PASSWORD}" <<'SQL' >/dev/null
SELECT set_config('claw.app_user', :'app_user', false);
SELECT set_config('claw.app_password', :'app_password', false);
DO $$
DECLARE
  app_user text := current_setting('claw.app_user');
  app_password text := current_setting('claw.app_password');
  db_name text := current_database();
  r record;
BEGIN
  IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = app_user) THEN
    EXECUTE format('CREATE ROLE %I LOGIN PASSWORD %L NOSUPERUSER NOBYPASSRLS NOCREATEROLE', app_user, app_password);
  ELSE
    EXECUTE format('ALTER ROLE %I LOGIN PASSWORD %L NOSUPERUSER NOBYPASSRLS NOCREATEROLE', app_user, app_password);
  END IF;

  EXECUTE format('ALTER DATABASE %I OWNER TO %I', db_name, app_user);
  EXECUTE format('ALTER SCHEMA public OWNER TO %I', app_user);
  EXECUTE format('GRANT USAGE, CREATE ON SCHEMA public TO %I', app_user);

  FOR r IN
    SELECT c.relkind, c.relname
    FROM pg_class c
    JOIN pg_namespace n ON n.oid = c.relnamespace
    WHERE n.nspname = 'public'
      AND c.relkind IN ('r', 'p', 'f')
    ORDER BY c.relname
  LOOP
    EXECUTE format('ALTER TABLE %I OWNER TO %I', r.relname, app_user);
  END LOOP;

  FOR r IN
    SELECT c.relkind, c.relname
    FROM pg_class c
    JOIN pg_namespace n ON n.oid = c.relnamespace
    WHERE n.nspname = 'public'
      AND c.relkind IN ('v', 'm')
    ORDER BY c.relname
  LOOP
    EXECUTE format('ALTER VIEW %I OWNER TO %I', r.relname, app_user);
  END LOOP;

  FOR r IN
    SELECT c.relname
    FROM pg_class c
    JOIN pg_namespace n ON n.oid = c.relnamespace
    WHERE n.nspname = 'public'
      AND c.relkind = 'S'
      AND NOT EXISTS (
        SELECT 1
        FROM pg_depend d
        WHERE d.objid = c.oid
          AND d.deptype IN ('a', 'i')
      )
    ORDER BY c.relname
  LOOP
    EXECUTE format('ALTER SEQUENCE %I OWNER TO %I', r.relname, app_user);
  END LOOP;
END $$;
SQL
    echo "OK — infra postgres ready (${PG_CTN}:${PG_PORT})" >&2
    echo "    CLAW_GATEWAY_DATABASE_URL=postgres://${APP_USER}:****@${PG_ADVERTISE_HOST}:${PG_PORT}/${CLAW_GATEWAY_PG_DATABASE:-claw_gateway}" >&2
    exit 0
  fi
  [[ "${i}" -eq 30 ]] && { echo "error: postgres not healthy after 60s" >&2; exit 1; }
  sleep 2
done

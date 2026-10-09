#!/usr/bin/env bash
# One-time pre-upgrade migration from fixed ACP pins to raw Agent engines. Author: kejiqing
set -euo pipefail

: "${CLAW_GATEWAY_DATABASE_URL:?CLAW_GATEWAY_DATABASE_URL is required}"
: "${OPENCODE_ENGINE_REF:?OPENCODE_ENGINE_REF is required}"
: "${OPENCODE_ENGINE_DIGEST:?OPENCODE_ENGINE_DIGEST is required}"
: "${APPSERVER_ENGINE_REF:?APPSERVER_ENGINE_REF is required}"
: "${APPSERVER_ENGINE_DIGEST:?APPSERVER_ENGINE_DIGEST is required}"
command -v psql >/dev/null 2>&1 || { echo "psql is required" >&2; exit 1; }

for ref in "$OPENCODE_ENGINE_REF" "$APPSERVER_ENGINE_REF"; do
  [[ "$ref" =~ ^https?://[^[:space:]]+\.tar\.gz$ ]] || {
    echo "Agent engine ref must be an http(s) tar.gz URL: $ref" >&2
    exit 1
  }
done
for digest in "$OPENCODE_ENGINE_DIGEST" "$APPSERVER_ENGINE_DIGEST"; do
  [[ "$digest" =~ ^sha256:[0-9a-fA-F]{64}$ ]] || {
    echo "Agent engine digest must be sha256:<64 hex>" >&2
    exit 1
  }
done

psql "$CLAW_GATEWAY_DATABASE_URL" \
  -v ON_ERROR_STOP=1 \
  -v opencode_ref="$OPENCODE_ENGINE_REF" \
  -v opencode_digest="$OPENCODE_ENGINE_DIGEST" \
  -v appserver_ref="$APPSERVER_ENGINE_REF" \
  -v appserver_digest="$APPSERVER_ENGINE_DIGEST" <<'SQL'
BEGIN;

UPDATE gateway_global_settings
SET settings_json =
      (settings_json - 'cliPins')
      || jsonb_build_object(
           'agentEngines',
           jsonb_build_object(
             'engines',
             jsonb_build_object(
               'opencode', jsonb_build_object(
                 'ref', :'opencode_ref',
                 'digest', :'opencode_digest'
               ),
               'appserver', jsonb_build_object(
                 'ref', :'appserver_ref',
                 'digest', :'appserver_digest'
               )
             )
           )
         ),
    updated_at_ms = (EXTRACT(EPOCH FROM clock_timestamp()) * 1000)::bigint;

DO $$
BEGIN
  IF NOT EXISTS (SELECT 1 FROM gateway_global_settings)
     OR EXISTS (
    SELECT 1
    FROM gateway_global_settings
    WHERE settings_json ? 'cliPins'
       OR jsonb_object_length(settings_json #> '{agentEngines,engines}') <> 2
  ) THEN
    RAISE EXCEPTION 'Agent engine configuration migration failed';
  END IF;
END
$$;

COMMIT;

SELECT cluster_id, settings_json -> 'agentEngines' AS agent_engines
FROM gateway_global_settings
ORDER BY cluster_id;
SQL

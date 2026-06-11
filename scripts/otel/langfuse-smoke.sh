#!/usr/bin/env bash
# OTLP smoke: export distributed solve trace to Langfuse. Author: kejiqing
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT/rust"

if [[ -f "$ROOT/.env" ]]; then
  set -a
  # shellcheck disable=SC1091
  source "$ROOT/.env"
  set +a
fi

if [[ "${CLAW_OTEL_ENABLED:-0}" != "1" ]]; then
  echo "CLAW_OTEL_ENABLED is not 1; set LANGFUSE_* in $ROOT/.env" >&2
  exit 1
fi

cargo run -p telemetry --example langfuse_smoke

BASE="${LANGFUSE_BASE_URL%/}"
PK="${LANGFUSE_PUBLIC_KEY:?LANGFUSE_PUBLIC_KEY required}"
SK="${LANGFUSE_SECRET_KEY:?LANGFUSE_SECRET_KEY required}"
AUTH="$(printf '%s:%s' "$PK" "$SK" | base64 | tr -d '\n')"

echo "Querying Langfuse traces (last 5)..."
curl -fsS -H "Authorization: Basic $AUTH" \
  "$BASE/api/public/traces?limit=5" | head -c 2000
echo

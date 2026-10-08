#!/usr/bin/env bash
# OTLP smoke: export distributed solve trace (any OTLP backend / SkyWalking). Author: kejiqing
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
  echo "CLAW_OTEL_ENABLED is not 1; set OTEL_EXPORTER_OTLP_ENDPOINT in $ROOT/.env" >&2
  exit 1
fi

if [[ -z "${OTEL_EXPORTER_OTLP_ENDPOINT:-}" ]]; then
  echo "OTEL_EXPORTER_OTLP_ENDPOINT is required" >&2
  exit 1
fi

cargo run -p telemetry --example otel_smoke

echo "OTLP smoke finished (check collector / SkyWalking UI for service claw-gateway-rs)."
echo "On export failure, grep worker/gateway logs: telemetry.otel.export_failed"

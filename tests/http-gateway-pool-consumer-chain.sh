#!/usr/bin/env bash
# Guard pool v1 consumer APIs: tools, progress, timeline read PG (not ephemeral worker disk).
# Author: kejiqing
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
RUST_DIR="${REPO_ROOT}/rust"

echo "==> [1/4] turn_tools_api unit tests"
cd "${RUST_DIR}"
cargo test -p http-gateway-rs list_turn_tools -- --nocapture

echo "==> [2/4] pool_consumer_resolve unit tests"
cargo test -p http-gateway-rs resolve_turn_progress_reads_pg -- --nocapture
cargo test -p http-gateway-rs resolve_turn_timeline_reads_pg -- --nocapture

if [[ -z "${CLAW_GATEWAY_TEST_DATABASE_URL:-}" ]]; then
  echo "==> [3/4] skip PG integration — set CLAW_GATEWAY_TEST_DATABASE_URL (./deploy/stack/gateway.sh pg-test-up)"
  echo "==> [4/4] skip tools PG chain integration"
  echo "pool consumer chain (unit-only) passed"
  exit 0
fi

echo "==> [3/4] tools PG import → render_session_jsonl → tools API"
cargo test -p http-gateway-rs tools_api_chain_pg_transcript_without_disk_jsonl -- --nocapture

echo "==> [4/4] done"
echo "pool consumer chain tests passed"

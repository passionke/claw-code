#!/usr/bin/env bash
# One-shot cleanup: legacy session_db integration test rows (fixed turn_id / session_id).
# Author: kejiqing
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
# shellcheck source=/dev/null
set -a && source "${ROOT}/.env" && set +a

VENV_PY="${ROOT}/deploy/e2b/.venv-pg/bin/python3"
if [[ ! -x "${VENV_PY}" ]]; then
  VENV_PY=python3
fi

exec "${VENV_PY}" <<PY
import os
import sys

root = ${ROOT@Q}
sys.path.insert(0, os.path.join(root, "deploy/e2b"))
url = os.environ.get("CLAW_GATEWAY_TEST_DATABASE_URL") or os.environ.get("CLAW_GATEWAY_DATABASE_URL")
if not url:
    print("error: set CLAW_GATEWAY_TEST_DATABASE_URL or CLAW_GATEWAY_DATABASE_URL", file=sys.stderr)
    sys.exit(1)

import psycopg

LEGACY_TURNS = (
    "T_a1b2c3d4e5f6478990abcdef12345678",
    "T_10000000000000000000000000000001",
    "T_20000000000000000000000000000002",
    "T_30000000000000000000000000000003",
    "T_a0000000000000000000000000000001",
    "T_b0000000000000000000000000000001",
    "T_c1000000000000000000000000000001",
    "T_d2000000000000000000000000000002",
    "T_a1000000000000000000000000000001",
    "T_b2000000000000000000000000000002",
)

print("Cleaning legacy gateway integration test rows from PG...")
with psycopg.connect(url) as conn:
    with conn.cursor() as cur:
        for table, col in (
            ("gateway_feedback", "turn_id"),
            ("gateway_session_artifacts", "turn_id"),
            ("gateway_turns", "turn_id"),
        ):
            cur.execute(
                f"DELETE FROM {table} WHERE {col} = ANY(%s)",
                (list(LEGACY_TURNS),),
            )
            print(f"  {table}: deleted {cur.rowcount} rows")
        cur.execute(
            "DELETE FROM gateway_sessions WHERE session_id = %s AND proj_id = %s",
            ("s1", 7),
        )
        print(f"  gateway_sessions: deleted {cur.rowcount} rows")
    conn.commit()
print("Done.")
PY

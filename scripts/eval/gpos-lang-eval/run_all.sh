#!/usr/bin/env bash
# GPOS 30x3 language eval: run cases then score. Author: kejiqing
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$ROOT"

export GATEWAY="${GATEWAY:-http://10.22.11.19:18088}"
export PROJ_ID="${PROJ_ID:-10}"
export CONCURRENCY="${CONCURRENCY:-4}"
export TIMEOUT_SEC="${TIMEOUT_SEC:-180}"
export POLL_SEC="${POLL_SEC:-5}"

ARGS=()
if [[ "${1:-}" == "--resume" ]]; then
  ARGS+=(--resume)
  shift
fi

echo "== run_eval.py gateway=$GATEWAY proj=$PROJ_ID concurrency=$CONCURRENCY =="
if ((${#ARGS[@]})); then
  python3 "$ROOT/run_eval.py" "${ARGS[@]}" "$@"
else
  python3 "$ROOT/run_eval.py" "$@"
fi

echo "== score.py =="
python3 "$ROOT/score.py"

echo "== summary =="
cat "$ROOT/results/summary.txt"

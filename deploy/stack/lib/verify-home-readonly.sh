#!/usr/bin/env bash
# Documented check: worker guest /claw_ds should be read-only (nasConfig readOnly). Author: kejiqing
# Run inside e2b worker after deploy; expects write to /claw_ds to fail.
set -euo pipefail

if [[ ! -d /claw_ds ]]; then
  echo "SKIP: /claw_ds not mounted (not inside e2b worker)"
  exit 0
fi

if touch /claw_ds/.claw-readonly-probe 2>/dev/null; then
  rm -f /claw_ds/.claw-readonly-probe
  echo "FAIL: /claw_ds is writable; home must be read-only for workers" >&2
  exit 1
fi

echo "OK: /claw_ds is not writable"

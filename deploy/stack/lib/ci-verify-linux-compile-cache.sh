#!/usr/bin/env bash
# Verify linux-compile-once cache restore/save + sccache from a GitHub Actions run.
# Usage: ci-verify-linux-compile-cache.sh [run-id]
# Author: kejiqing
set -euo pipefail

RUN_ID="${1:-}"
if [[ -z "${RUN_ID}" ]]; then
  echo "usage: $0 <github-actions-run-id>" >&2
  echo "  gh run list --workflow claw-code-image.yaml --limit 1" >&2
  exit 1
fi

if ! command -v gh >/dev/null 2>&1; then
  echo "error: gh CLI required" >&2
  exit 1
fi

LOG="$(mktemp)"
trap 'rm -f "${LOG}"' EXIT
gh run view "${RUN_ID}" --log > "${LOG}" 2>/dev/null || {
  echo "error: could not fetch run ${RUN_ID}" >&2
  exit 1
}

FAIL=0
pass() { echo "OK  $*"; }
fail() { echo "FAIL $*"; FAIL=1; }

if rg -q "Cache not found for input keys: linux-compile" "${LOG}"; then
  fail "actions/cache restore miss (expected on first warm or after lockfile change)"
else
  pass "actions/cache restore hit or partial restore"
fi

if rg -q "Failed to save.*tar.*failed|Permission denied" "${LOG}"; then
  fail "actions/cache save failed (check .ci-cache chown)"
else
  pass "no cache save permission errors in log"
fi

if rg -q "ci-cache ownership ok" "${LOG}"; then
  pass "workflow ownership verification step passed"
else
  fail "missing 'ci-cache ownership ok' step output"
fi

HITS="$(rg "Cache hits rate" "${LOG}" | tail -1 | rg -o '[0-9]+\.[0-9]+ %' | head -1 || true)"
if [[ -z "${HITS}" ]]; then
  fail "sccache stats not found in log"
elif [[ "${HITS}" == "0.00 %" ]]; then
  fail "sccache hit rate 0% (cold compile or cache not restored)"
else
  pass "sccache hit rate ${HITS}"
fi

RELEASE_TIME="$(rg "Finished \`release\` profile" "${LOG}" | tail -1 | rg -o 'in [0-9m s]+' || true)"
if [[ -n "${RELEASE_TIME}" ]]; then
  echo "INFO release compile ${RELEASE_TIME}"
  if echo "${RELEASE_TIME}" | rg -q 'in [0-9]+m [0-9]+s' && ! echo "${RELEASE_TIME}" | rg -q 'in [0-2]m '; then
    echo "WARN release compile > 2min (may be cold start — rerun after cache warm)"
  fi
else
  fail "release compile timing not found"
fi

if rg -q "pull compile image ghcr.io" "${LOG}"; then
  pass "used GHCR prebuilt claw-rust-compile pull"
elif rg -q "building compile image" "${LOG}"; then
  echo "WARN built compile image locally (GHCR pull miss or first publish)"
fi

exit "${FAIL}"

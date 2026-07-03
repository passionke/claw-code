#!/usr/bin/env bash
# Full pre-prod e2b path: templates (local) → singletons (PG) → hint gateway up. Author: kejiqing
set -euo pipefail

LIB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${LIB_DIR}/../.." && pwd)"

skip_templates=0
skip_singletons=0
template_args=()
singleton_args=(--reuse)

while [[ $# -gt 0 ]]; do
  case "$1" in
    --skip-templates) skip_templates=1; shift ;;
    --skip-singletons) skip_singletons=1; shift ;;
    --reset) singleton_args=(--reset); shift ;;
    --skip-cache) template_args+=(--skip-cache); shift ;;
    -h|--help)
      cat <<'EOF'
Usage: ./deploy/stack/gateway.sh e2b-pre-bootstrap [options]

  1) build-selfhosted-templates.sh  (local .venv-fc → e2b API docker build)
  2) e2b-singletons-up --reuse      (nas-api / ovs / observe → PG)
  3) print gateway up --release hint

Options:
  --skip-templates    singletons only (templates already on e2bserver)
  --skip-singletons   templates only
  --skip-cache        fresh template docker build
  --reset             recreate singleton sandboxes
EOF
      exit 0
      ;;
    *) echo "unknown: $1" >&2; exit 1 ;;
  esac
done

BUILD_SH="${REPO_ROOT}/deploy/e2b/build-selfhosted-templates.sh"
if [[ "${skip_templates}" -eq 0 ]]; then
  if [[ -f "${BUILD_SH}" ]]; then
    "${BUILD_SH}" "${template_args[@]}"
  else
    echo "==> skip templates (no ${BUILD_SH}; run on claw-code dev machine)" >&2
    echo "    cd ~/work/claw-code && ./deploy/e2b/build-selfhosted-templates.sh" >&2
  fi
fi

if [[ "${skip_singletons}" -eq 0 ]]; then
  "${LIB_DIR}/e2b-singletons-up.sh" "${singleton_args[@]}"
fi

echo "==> pre-bootstrap done; start gateway on deploy host:" >&2
echo "    cd ~/work/claw-deploy && ./deploy/stack/gateway.sh up --release release-vX.Y.Z" >&2

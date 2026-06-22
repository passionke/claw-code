#!/usr/bin/env bash
# Start claude-tap Live on e2b — direct traffic URL, no gateway. Author: kejiqing
set -euo pipefail

LIB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${LIB_DIR}/../../.." && pwd)"
ENV_FILE="${REPO_ROOT}/.env"

if [[ ! -f "${ENV_FILE}" ]]; then
  echo "missing ${ENV_FILE} — copy from deploy/stack/env.selfhosted-e2b.example" >&2
  exit 1
fi

# shellcheck disable=SC1090
set -a && source "${ENV_FILE}" && set +a

if ! python3 -c "import e2b_code_interpreter" 2>/dev/null; then
  FC_VENV="${REPO_ROOT}/.venv-fc"
  if [[ ! -x "${FC_VENV}/bin/python3" ]]; then
    echo "==> creating ${FC_VENV} (e2b-code-interpreter)" >&2
    python3 -m venv "${FC_VENV}"
    "${FC_VENV}/bin/pip" install -q e2b-code-interpreter
  fi
  export PATH="${FC_VENV}/bin:${PATH}"
fi

if ! python3 -c "import e2b_code_interpreter" 2>/dev/null; then
  echo "error: pip install e2b-code-interpreter (or fix .venv-fc)" >&2
  exit 1
fi

exec python3 "${REPO_ROOT}/deploy/fc-sandbox/fc-tap-live-up.py" "$@"

#!/usr/bin/env bash
# Private Jenkins entry: publish e2b Worker protocol images to Nora. Author: kejiqing
# Never builds Gateway/Admin or Agent engine artifacts.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
bash "$ROOT/deploy/verify-release-boundaries.sh"

RELEASE_TAG="${RELEASE_TAG:-${GIT_TAG:-}}"
if [[ -z "$RELEASE_TAG" ]]; then
  echo "RELEASE_TAG or GIT_TAG is required" >&2
  exit 2
fi

export RELEASE_TAG
export REGION="${REGION:-china}"
export CLAW_IMAGE_PREFIX="${CLAW_IMAGE_PREFIX:-nora.home.passionke.top/passionke}"
export CLAW_LINUX_COMPILE_PLATFORM="${CLAW_LINUX_COMPILE_PLATFORM:-linux/amd64}"

# Private home series only. Do not fall back to bare 10.8.0.1. Author: kejiqing
export CLAW_E2B_API_URL="${CLAW_E2B_API_URL:-http://e2b.home.passionke.top:3000}"
export CLAW_E2B_SANDBOX_URL="${CLAW_E2B_SANDBOX_URL:-http://e2b.home.passionke.top:3002}"
export E2B_API_URL="${E2B_API_URL:-$CLAW_E2B_API_URL}"
export E2B_SANDBOX_URL="${E2B_SANDBOX_URL:-$CLAW_E2B_SANDBOX_URL}"

: "${NEXUS_USER:?NEXUS_USER is required (Jenkins nora-deployer)}"
: "${NEXUS_PASSWORD:?NEXUS_PASSWORD is required (Jenkins nora-deployer)}"
export NEXUS_USER NEXUS_PASSWORD
export CLAW_REGISTRY_USER="${CLAW_REGISTRY_USER:-$NEXUS_USER}"
export CLAW_REGISTRY_PASSWORD="${CLAW_REGISTRY_PASSWORD:-$NEXUS_PASSWORD}"

# home29 has Python 3.14 without pip/venv. Run pinned e2b SDK in a mirrored
# CPython container; host docker still builds/pushes protocol images.
# Author: kejiqing
E2B_PY_IMAGE="${E2B_PY_IMAGE:-docker.m.daocloud.io/library/python:3.12-slim}"
WRAPPER="${ROOT}/.ci-e2b-python.sh"
python3 - "$ROOT" "$E2B_PY_IMAGE" "$WRAPPER" <<'PY'
import pathlib
import sys

root, image, path = sys.argv[1], sys.argv[2], sys.argv[3]
pathlib.Path(path).write_text(
    f"""#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR={root!r}
IMAGE={image!r}
REQ=\"${{ROOT_DIR}}/deploy/e2b/requirements-e2b-sdk.txt\"
args=()
while IFS= read -r line; do
  key=${{line%%=*}}
  case \"$key\" in
    CLAW_*|E2B_*|PG*|DATABASE_URL|REGION) args+=(-e \"$line\") ;;
  esac
done < <(env)
exec docker run --rm --network host \\
  -v \"${{ROOT_DIR}}:${{ROOT_DIR}}:rw\" \\
  -w \"${{ROOT_DIR}}\" \\
  -e HOME=/tmp \\
  \"${{args[@]}}\" \\
  \"${{IMAGE}}\" \\
  bash -lc 'set -euo pipefail; pip install -q -i https://mirrors.aliyun.com/pypi/simple --trusted-host mirrors.aliyun.com -r \"$0\"; exec python \"$@\"' \\
  \"$REQ\" \"$@\"
""",
    encoding="utf-8",
)
PY
chmod +x "$WRAPPER"
export CLAW_E2B_PYTHON="$WRAPPER"
unset CLAW_E2B_VENV || true

exec bash "$ROOT/deploy/e2b/publish-worker-protocol.sh"

#!/usr/bin/env bash
# Jenkins claw-code job entry — thin forward to ONE path. Author: kejiqing.
# Do not reintroduce a second bake/publish implementation here.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
exec "$ROOT/deploy/pack/publish.sh" all "$@"

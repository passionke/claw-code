#!/usr/bin/env bash
# Deprecated: use deploy/stack/lib/install-pool-daemon-from-image.sh (compose 侧车为常态). Author: kejiqing
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
exec bash "${ROOT}/deploy/stack/lib/install-pool-daemon-from-image.sh" "$@"

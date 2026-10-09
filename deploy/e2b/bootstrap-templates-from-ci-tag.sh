#!/usr/bin/env bash
# REMOVED — no more extract+COPY claw / engine Template.build. Author: kejiqing.
# ONE path: deploy/pack/publish.sh e2b-register (from_image empty shells only).
# CLI versions: Admin → Worker CLI pins + worker.init inject.
set -euo pipefail
echo "REMOVED: debian+COPY / engine template bake is gone." >&2
echo "  Empty shell register: RELEASE_TAG=<tag> ./deploy/pack/publish.sh e2b-register" >&2
echo "  CLI: publish.sh cli-* then Admin PUT cli-pins (not this script)." >&2
exit 2

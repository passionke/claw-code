# shellcheck shell=bash
# Deprecated: source deploy/stack/lib/compose-include.sh instead. Author: kejiqing
_DEPLOY_PODMAN_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck disable=SC1091
source "${_DEPLOY_PODMAN_DIR}/../stack/lib/compose-include.sh"

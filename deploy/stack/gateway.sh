#!/usr/bin/env bash
set -euo pipefail

# Single entrypoint for gateway deploy; implementations live in ./lib/. Author: kejiqing
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
LIB="${SCRIPT_DIR}/lib"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"

usage() {
  cat <<'EOF'
Usage:
  ./deploy/stack/gateway.sh <command>

Commands:
  clean         Remove rust/target + deploy/stack/.linux-artifacts (optional --workspace)
  build         clean (default) + build images (Darwin: podman run compile; log: .build.log)
  pack-deploy   Build images + restart stack (local standard; log: .build.log)
  solve-once-local  Host-side one-turn gateway-solve-once (no worker container)
  up            Start/recreate gateway + pool only (does not stop/start postgres)
  down          Stop gateway + pool only (postgres keeps running)
  pg-up         Start postgres only
  pg-down       Stop postgres only (data volume kept)
  restart       Recreate gateway stack (down + up)
  check         Connectivity smoke check
  tap-up        Start claude-tap only (see CLAUDE_TAP_MODE in .env)
  tap-down      Stop claude-tap only
  build-tap     Build claude-tap image from CLAUDE_TAP_BUILD_CONTEXT (fork)
  bench         Pool bench 30s
  logs          Follow gateway logs
  ps            Show relevant containers
  e2e           Run tests/http-gateway-session-continuity-e2e.sh
  help          Show this help

Implementation scripts: deploy/stack/lib/ (do not run directly unless you know why).
EOF
}

cmd="${1:-help}"
shift || true

case "${cmd}" in
  clean) "${LIB}/clean.sh" "$@" ;;
  build) "${LIB}/build.sh" "$@" ;;
  pack-deploy) "${LIB}/pack-deploy.sh" "$@" ;;
  solve-once-local) "${LIB}/solve-once-local.sh" "$@" ;;
  up) "${LIB}/up.sh" "$@" ;;
  down) "${LIB}/down.sh" "$@" ;;
  pg-up) "${LIB}/pg-up.sh" "$@" ;;
  pg-down) "${LIB}/pg-down.sh" "$@" ;;
  restart) "${LIB}/down.sh" && "${LIB}/up.sh" "$@" ;;
  check) "${LIB}/check-connectivity.sh" "$@" ;;
  tap-up) bash "${LIB}/tap-up.sh" "$@" ;;
  tap-down) bash "${LIB}/tap-down.sh" "$@" ;;
  build-tap)
    set -a
    # shellcheck disable=SC1090
    [[ -f "${REPO_ROOT}/.env" ]] && source "${REPO_ROOT}/.env"
    set +a
    # shellcheck source=/dev/null
    source "${LIB}/compose-include.sh"
    # shellcheck source=/dev/null
    source "${LIB}/claude-tap-local.sh"
    ctx="$(claw_claude_tap_resolve_context "${REPO_ROOT}")"
    rt="$(claw_container_runtime_cli)"
    claw_claude_tap_build_image "${rt}" "${ctx}" "${CLAUDE_TAP_IMAGE:-claude-tap:local}"
    ;;
  bench) "${LIB}/bench-pool-30s.sh" "$@" ;;
  logs)
    podman logs -f claw-gateway-rs
    ;;
  ps)
    rt="$(command -v docker >/dev/null 2>&1 && echo docker || echo podman)"
    "${rt}" ps --format 'table {{.Names}}\t{{.Image}}\t{{.Status}}\t{{.Ports}}' \
      | rg 'claw-gateway|claw-worker|claw-gw-|claw-claude-tap|sqlbot|NAMES' || true
    ;;
  e2e)
    "${REPO_ROOT}/tests/http-gateway-session-continuity-e2e.sh" "$@"
    ;;
  help|-h|--help) usage ;;
  *)
    echo "unknown command: ${cmd}" >&2
    usage
    exit 2
    ;;
esac

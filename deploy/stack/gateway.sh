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
  build         Build gateway + worker images; full log → deploy/stack/.build.log (see: build --help)
  up            Start/recreate gateway stack (`up --release TAG` = down + kill pool + rm all workers + pull + up)
  down          Stop gateway stack
  restart       Recreate stack (down + up)
  check         Connectivity smoke check
  tap-up        Start gateway + claude-tap (see CLAUDE_TAP_MODE in .env)
  tap-down      Stop gateway + claude-tap
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
  build) "${LIB}/build.sh" "$@" ;; # pass --log PATH | --no-log | IMAGE_TAG
  up) "${LIB}/up.sh" "$@" ;;
  down) "${LIB}/down.sh" "$@" ;;
  restart) "${LIB}/down.sh" && "${LIB}/up.sh" "$@" ;;
  check) "${LIB}/check-connectivity.sh" "$@" ;;
  tap-up) "${LIB}/start-with-tap.sh" "$@" ;;
  tap-down) "${LIB}/stop-with-tap.sh" "$@" ;;
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

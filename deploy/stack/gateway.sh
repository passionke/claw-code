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
  build         Build gateway + worker images (single step; always run both after Rust changes)
  up            Start/recreate gateway stack
  down          Stop gateway stack
  restart       Recreate stack (down + up)
  check         Connectivity smoke check
  tap-up        Start gateway + claude-tap
  tap-down      Stop gateway + claude-tap
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
  build) "${LIB}/build.sh" "$@" ;;
  up) "${LIB}/up.sh" "$@" ;;
  down) "${LIB}/down.sh" "$@" ;;
  restart) "${LIB}/down.sh" && "${LIB}/up.sh" ;;
  check) "${LIB}/check-connectivity.sh" "$@" ;;
  tap-up) "${LIB}/start-with-tap.sh" "$@" ;;
  tap-down) "${LIB}/stop-with-tap.sh" "$@" ;;
  bench) "${LIB}/bench-pool-30s.sh" "$@" ;;
  logs)
    podman logs -f claw-gateway-rs
    ;;
  ps)
    "${rt}" ps --format 'table {{.Names}}\t{{.Image}}\t{{.Status}}\t{{.Ports}}' \
      | rg 'claw-gateway-rs|claw-worker-|claw-gw-|claw-claude-tap|sqlbot|NAMES' || true
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

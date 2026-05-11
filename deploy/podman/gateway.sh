#!/usr/bin/env bash
set -euo pipefail

# Single entrypoint for local gateway deploy workflow. Author: kejiqing
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"

usage() {
  cat <<'EOF'
Usage:
  ./deploy/podman/gateway.sh <command>

Commands:
  build         Build gateway + worker images
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
EOF
}

cmd="${1:-help}"
shift || true

build_worker_image() {
  (
    cd "${REPO_ROOT}"
    set -a
    [[ -f .env ]] && source ./.env
    set +a
    # shellcheck disable=SC1091
    source ./deploy/podman/compose-include.sh
    local reg cli
    reg="${CONTAINER_BASE_REGISTRY:-docker.1ms.run}"
    cli="$(claw_container_runtime_cli)"
    "${cli}" build \
      --build-arg "RUST_BASE_IMAGE=${reg}/library/rust:1.88-bookworm" \
      --build-arg "DEBIAN_BASE_IMAGE=${reg}/library/debian:bookworm-slim" \
      -f deploy/podman/Containerfile.gateway-worker \
      -t claw-gateway-worker:local .
  )
}

case "${cmd}" in
  build)
    "${SCRIPT_DIR}/build.sh" "$@"
    build_worker_image
    ;;
  up) "${SCRIPT_DIR}/up.sh" "$@" ;;
  down) "${SCRIPT_DIR}/down.sh" "$@" ;;
  restart) "${SCRIPT_DIR}/down.sh" && "${SCRIPT_DIR}/up.sh" ;;
  check) "${SCRIPT_DIR}/check-connectivity.sh" "$@" ;;
  tap-up) "${SCRIPT_DIR}/start-with-tap.sh" "$@" ;;
  tap-down) "${SCRIPT_DIR}/stop-with-tap.sh" "$@" ;;
  bench) "${SCRIPT_DIR}/bench-pool-30s.sh" "$@" ;;
  logs)
    podman logs -f claw-gateway-rs
    ;;
  ps)
    podman ps --format 'table {{.Names}}\t{{.Image}}\t{{.Status}}\t{{.Ports}}' \
      | rg 'claw-gateway-rs|claw-gw-|claw-claude-tap|sqlbot|NAMES' || true
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

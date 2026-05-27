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
  quick         Daily local stack: host pool-daemon + fast playground image + pool-reset + up + check
  clean         Remove rust/target (or --debug-only) + .linux-artifacts; optional podman cache/images
  build         clean (default) + build images (Darwin: podman run compile; log: .build.log)
  pack-deploy   Build gateway images + restart stack (slow; after Rust/image changes; log: .build.log)
  playground    Run host playground UI (solve_async + /admin; builds admin dist first)
  admin-build   Local only: gateway-admin dist (needs Node>=18; set CLAW_GATEWAY_ADMIN_LOCAL_BUILD=1)
  admin-reload  Local only: admin-build + copy dist into playground container (not for --release servers)
  solve-once-local  Host-side one-turn gateway-solve-once (no worker container)
  up            Start/recreate gateway + pool only (does not stop/start postgres)
  down          Stop gateway + pool only (postgres keeps running)
  pg-up         Start postgres only
  pg-down       Stop postgres only (data volume kept)
  restart       Recreate gateway stack (down + up)
  pool-reset    Stop host pool daemon + remove all claw-worker containers
  check         Connectivity smoke check
  verify        Stack truth checks (schema, pool registry, binary); fails loud
  tap-up        Start claude-tap only (CLAUDE_TAP_MODE: native/pypi=PyPI claw-tap, docker=image)
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
  quick) "${LIB}/quick.sh" "$@" ;;
  clean) "${LIB}/clean.sh" "$@" ;;
  build) "${LIB}/build.sh" "$@" ;;
  pack-deploy) "${LIB}/pack-deploy.sh" "$@" ;;
  playground) "${LIB}/playground.sh" "$@" ;;
  admin-build) "${LIB}/build-gateway-admin.sh" "$@" ;;
  admin-reload) "${LIB}/admin-reload.sh" "$@" ;;
  solve-once-local) "${LIB}/solve-once-local.sh" "$@" ;;
  up) "${LIB}/up.sh" "$@" ;;
  down) "${LIB}/down.sh" "$@" ;;
  pg-up) "${LIB}/pg-up.sh" "$@" ;;
  pg-down) "${LIB}/pg-down.sh" "$@" ;;
  restart) "${LIB}/down.sh" && "${LIB}/up.sh" "$@" ;;
  pool-reset) "${LIB}/pool-reset.sh" "$@" ;;
  check) "${LIB}/check-connectivity.sh" "$@" ;;
  verify) "${LIB}/claw-stack-verify.sh" "$@" ;;
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
    rt="$(command -v podman >/dev/null 2>&1 && echo podman || echo docker)"
    "${rt}" logs -f claw-gateway-rs
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

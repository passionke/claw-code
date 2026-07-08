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
  quick         Daily local stack: playground image + up + check (e2b-only)
  clean         Remove rust/target (or --debug-only) + .linux-artifacts; optional podman cache/images
  build         clean (default) + build images (Darwin: podman run compile; log: .build.log)
  pack-deploy   Build gateway images + restart stack (slow; after Rust/image changes; log: .build.log)
  e2b-worker-deploy  Local dev: amd64 compile + e2b worker template (no CI/ACR)
  playground    Run host playground UI (solve_async + /admin; builds admin dist first)
  admin-build   Local only: gateway-admin dist (needs Node>=18; set CLAW_GATEWAY_ADMIN_LOCAL_BUILD=1)
  admin-reload  Local only: admin-build + copy dist into playground container (not for --release servers)
  solve-once-local  Host-side one-turn gateway-solve-once (no worker container)
  up            Start/recreate gateway + pool only (does not stop/start postgres)
  down          Stop gateway + pool only (postgres keeps running)
  pg-up         Start postgres only
  pg-down       Stop postgres only (data volume kept)
  restart       Recreate gateway stack (down + up)
  pool-reset    REMOVED — local podman pool deleted; use e2b (CLAW_INTERACTIVE_BACKEND=e2b)
  stable-dev-up Start dev-stable PG+tap on Linux host (.env.dev-stable; see env.stable-dev-host.example)
  pool-up       REMOVED — use CLAW_INTERACTIVE_BACKEND=e2b
  fix-workspace chown ds_* sessions + pool slots to CLAW_WORKER_UID (before up if preflight failed)
  install-docker  Linux production: apt/dnf install docker.io + compose + registry mirror (idempotent)
  check         Connectivity smoke check (auto pool-up if HTTP down)
  solve-e2e     Admin-equivalent solve_async + poll to succeeded/failed (real gate, not healthz)
  verify        Stack truth checks (schema, pool registry, binary); fails loud
  cluster-verify  Shared-PG multi-host: claw_pool zombies + each gateway /v1/pools (pre-prod gate)
  ovs-up        Ensure e2b OVS singleton (gateway API; gateway must be up)
  observe-tap-up Ensure e2b observe singleton (gateway API)
  nas-api-up    Ensure e2b claw-nas-api singleton (gateway API)
  e2b-singletons-up  nas-api + ovs + observe via gateway API (--reset to recreate)
  sync-e2b-env     Apply .env anchors → e2bserver panel/worker config (--restart --nginx)
  e2b-pre-bootstrap  build templates (local) then singletons → PG; then gateway up --release
  pre-252-e2b-up     pre-prod 252: preflight → templates → up --release → verify
  tap-down      Stop pool claude-tap only (legacy compose; e2b mode uses CLAUDE_TAP_MODE=off)
  build-tap     Build claude-tap image from CLAUDE_TAP_BUILD_CONTEXT (fork)
  bench         REMOVED — local pool bench deleted
  logs          Follow gateway logs
  ps            Show relevant containers
  e2e           Run tests/http-gateway-session-continuity-e2e.sh
  help          Show this help

Implementation scripts: deploy/stack/lib/ (singleton lifecycle: gateway-rs + Admin API).
EOF
}

print_deploy_manual_hint() {
  cat >&2 <<'EOF'
manual: deployment troubleshooting
  - deploy/stack/README.md
  - 重点先看: "1. 稳定路径（按顺序做）", "1.3 启动与检查", "3. 常见问题（短）"
  - 若是权限门禁报错，再按报错里的 hint 执行 chown 修复
EOF
}

run_with_manual_hint() {
  local runner="${1:?}"
  shift || true
  if ! "${runner}" "$@"; then
    print_deploy_manual_hint
    return 1
  fi
}

cmd="${1:-help}"
shift || true

case "${cmd}" in
  quick) run_with_manual_hint "${LIB}/quick.sh" "$@" ;;
  clean) "${LIB}/clean.sh" "$@" ;;
  build) run_with_manual_hint "${LIB}/build.sh" "$@" ;;
  pack-deploy) run_with_manual_hint "${LIB}/pack-deploy.sh" "$@" ;;
  e2b-worker-deploy) run_with_manual_hint "${LIB}/e2b-worker-deploy.sh" "$@" ;;
  playground) "${LIB}/playground.sh" "$@" ;;
  admin-build) "${LIB}/build-gateway-admin.sh" "$@" ;;
  admin-reload) "${LIB}/admin-reload.sh" "$@" ;;
  solve-once-local) "${LIB}/solve-once-local.sh" "$@" ;;
  up) run_with_manual_hint "${LIB}/up.sh" "$@" ;;
  down) "${LIB}/down.sh" "$@" ;;
  pg-up) "${LIB}/pg-up.sh" "$@" ;;
  pg-down) "${LIB}/pg-down.sh" "$@" ;;
  restart)
    run_with_manual_hint "${LIB}/down.sh"
    run_with_manual_hint "${LIB}/up.sh" "$@"
    ;;
  pool-reset)
    echo "error: pool-reset removed (local claw-sandbox pool deleted; use CLAW_INTERACTIVE_BACKEND=e2b)" >&2
    exit 1
    ;;
  stable-dev-up)
    echo "error: stable-dev-up removed (host claw-sandbox pool deleted; use infra-pg + FC)" >&2
    exit 1
    ;;
  pool-up)
    echo "error: pool-up removed (use CLAW_INTERACTIVE_BACKEND=e2b)" >&2
    exit 1
    ;;
  fix-workspace) "${LIB}/fix-session-ownership.sh" "$@" ;;
  install-docker) "${LIB}/install-docker.sh" "$@" ;;
  check) "${LIB}/check-connectivity.sh" "$@" ;;
  solve-e2e) "${LIB}/admin-solve-e2e.sh" "$@" ;;
  verify) "${LIB}/claw-stack-verify.sh" "$@" ;;
  cluster-verify) "${LIB}/claw-cluster-verify.sh" "$@" ;;
  ovs-up) bash "${LIB}/e2b-ovs-up.sh" "$@" ;;
  observe-tap-up) bash "${LIB}/e2b-tap-live-up.sh" "$@" ;;
  nas-api-up) bash "${LIB}/e2b-nas-api-up.sh" "$@" ;;
  e2b-singletons-up) bash "${LIB}/e2b-singletons-up.sh" "$@" ;;
  sync-e2b-env) bash "${LIB}/sync-e2b-host-env.sh" "$@" ;;
  e2b-pre-bootstrap) bash "${LIB}/e2b-pre-bootstrap.sh" "$@" ;;
  pre-252-e2b-up) bash "${LIB}/pre-252-e2b-pipeline.sh" "$@" ;;
  tap-up)
    echo "error: tap-up removed (FC mode: use ./deploy/stack/gateway.sh observe-tap-up)" >&2
    exit 1
    ;;
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
  bench)
    echo "error: bench removed (local pool deleted)" >&2
    exit 1
    ;;
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

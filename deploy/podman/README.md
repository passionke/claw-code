# Podman Deployment (Gateway + Claude-Tap Proxy)

中文快速上手请看：`docs/http-gateway-rs-quickstart.md`

This deployment runs two processes:
- `gateway-rs` in podman container
- `claude-tap` on host as API proxy/trace viewer

`claude-tap` is not an MCP server. It proxies model API traffic and records traces.

## 1) Build gateway image

```bash
./deploy/podman/build.sh
```

- **Local default**: `CONTAINER_BASE_REGISTRY=docker.1ms.run` (set in repo-root `.env` or the environment; `build.sh` loads `.env` automatically). Same variable name as GitHub Actions for the image workflow.
- **docker.io**: set `CLAW_USE_DOCKER_IO=1` (or run on GitHub Actions, where `GITHUB_ACTIONS=true` is set automatically).

Optional: build with a specific tag (for release deployment):

```bash
./deploy/podman/build.sh release-v1.0.8
```

### Worker image (container pool)

When `CLAW_SOLVE_ISOLATION=docker_pool` / `podman_pool`, the gateway expects **`CLAW_DOCKER_IMAGE`** / **`CLAW_PODMAN_IMAGE`** to point at a long‑lived worker (default entrypoint: `scripts/claw-gateway-worker.sh` → `sleep infinity`; solve runs via `docker exec … claw gateway-solve-once`).

Build from the **repository root**:

```bash
# Uses the same CONTAINER_BASE_REGISTRY / CLAW_USE_DOCKER_IO as build.sh if you export them or `set -a; source .env; set +a`
REG="${CONTAINER_BASE_REGISTRY:-docker.1ms.run}"
podman build \
  --build-arg "RUST_BASE_IMAGE=${REG}/library/rust:1.88-bookworm" \
  --build-arg "DEBIAN_BASE_IMAGE=${REG}/library/debian:bookworm-slim" \
  -f deploy/podman/Containerfile.gateway-worker \
  -t claw-gateway-worker:local .
```

The gateway bind‑mounts **`CLAW_WORK_ROOT`** at **`/claw_host_root`** inside each worker; see `docs/http-gateway-container-pool.md` §6.1–6.3.

### Local compose (default = `podman_pool`)

1. Build **gateway** and **worker** images (`./deploy/podman/build.sh` and worker `podman build` from README above).
2. Repo-root `.env` (see `.env.example`): **`PODMAN_HOST_SOCK`** is required unless you set **`CLAW_SOLVE_ISOLATION=inprocess`**. Defaults include pool size / worker image.
3. `./deploy/podman/up.sh` or `start-with-tap.sh` sets `CLAW_POOL_WORK_ROOT_HOST` and merges `podman-compose.podman-api.yml` whenever mode is not `inprocess`.

### Local Podman vs remote Docker (same gateway binary)

| Where | `CLAW_SOLVE_ISOLATION` | Runtime CLI | Env prefix | Socket / access |
| --- | --- | --- | --- | --- |
| **This compose stack (dev laptop)** | `podman_pool` (default) | `podman` (in image) | `CLAW_PODMAN_*` | `PODMAN_HOST_SOCK` → `podman-compose.podman-api.yml` |
| **Remote / server (Docker Engine)** | `docker_pool` | `docker` | `CLAW_DOCKER_*` | Mount `docker.sock` or set `DOCKER_HOST`; image must include **`docker` CLI** (this Podman-focused image installs `podman` only—use a small Docker-client layer or host-run gateway for `docker_pool`) |

Worker image name differs only by env: `CLAW_PODMAN_IMAGE` vs `CLAW_DOCKER_IMAGE`. Pool sizing / caps (`CLAW_POOL_SIZE_CAP`, `POOL_SIZE`, `POOL_MIN_IDLE`, `POOL_CPUS`, `POOL_MEMORY`) use the same **per-runtime** prefix (`CLAW_PODMAN_…` / `CLAW_DOCKER_…`) as in `docs/http-gateway-container-pool.md`.

## 2) Configure env (repo root only)

```bash
cp .env.example .env
```

Edit **repository root** `.env`:

- `GATEWAY_IMAGE`, `GATEWAY_HOST_PORT`
- `OPENAI_API_KEY`, `OPENAI_BASE_URL` (e.g. claude-tap: `http://host.containers.internal:8080/v1`)
- `INTERNAL_CLAUDE_TAP_HOST` (for docs/scripts; gateway does not inject `OPENAI_BASE_URL` automatically)
- `CLAW_HOST_LOG_DIR` (default `./deploy/podman/claw-logs` — create the directory if missing)
- optional `CLAW_LOG_LEVEL`, `CLAW_TRACE_ENABLED`
- for `start-with-tap.sh`: `UPSTREAM_OPENAI_BASE_URL`, `CLAUDE_TAP_PORT`, `CLAUDE_TAP_LIVE_PORT`

Mount **repository root** `.claw.json` at `/app/.claw.json` (see `podman-compose.yml`). The gateway applies `.claw.json` `env` (when unset) and `model` (after `CLAW_DEFAULT_MODEL`) on each solve. `up.sh` / `start-with-tap.sh` **only create an empty `{}` if the file is missing** — they do not overwrite your local `.claw.json`.

`podman-compose.yml` also loads `deploy/podman/gateway-allowlist.env` after the root `.env` so `CLAW_ALLOWED_TOOLS` can override an empty `CLAW_ALLOWED_TOOLS=` in the root file (Compose `environment:` cannot fix that). Edit that file to test per-request `allowedTools` against a global allowlist.

## 3) Start (gateway + claude-tap)

```bash
./deploy/podman/start-with-tap.sh
```

## 4) Verify connectivity chain

```bash
./deploy/podman/check-connectivity.sh
```

This script validates:
- gateway `/healthz`
- async solve path
- default MCP wiring (`CLAW_DEFAULT_HTTP_MCP_NAME`) inside gateway

## 5) Stop

```bash
./deploy/podman/stop-with-tap.sh
```

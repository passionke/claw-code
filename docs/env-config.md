# Environment configuration (inventory + two profiles)

Author: kejiqing

## Two supported profiles

Set **one** knob in repo root `.env`:

| `CLAW_DEPLOY_PROFILE` | OS / runtime | Worker backend | Images | Start |
| --- | --- | --- | --- | --- |
| **`local`** (default on macOS) | Podman + macOS | **e2b** | `pack-deploy local` → `:local` tags | `./deploy/stack/gateway.sh quick` |
| **`production`** (default on Linux) | Docker + Linux | **e2b** | CI only: `up --release release-vX.Y.Z` | `./deploy/stack/gateway.sh up --release release-vX.Y.Z` |

Scripts apply defaults via `deploy/stack/lib/env-profile.sh` after sourcing `.env`:
- `CLAW_INTERACTIVE_BACKEND=e2b`
- `CLAW_SOLVE_ISOLATION=e2b`

Copy-paste starters:

- **Recommended:** `deploy/stack/env.selfhosted-e2b.example` → `.env`
- `deploy/stack/env.local.example` → merge into `.env`
- `deploy/stack/env.production.example` → merge into `.env`
- e2b overlay: `deploy/stack/env.e2b-interactive.example`

---

## Human-maintained (repo root `.env`)

### Required everywhere

| Variable | Purpose |
| --- | --- |
| `CLAW_DEPLOY_PROFILE` | `local` or `production` (optional: auto from OS) |
| `CLAW_CLUSTER_ID` | Cluster id for PG row scoping |
| `CLAW_GATEWAY_DATABASE_URL` | External PG (recommended) or in-stack `postgres:5432` |
| `CLAW_E2B_API_URL` / `CLAW_E2B_SANDBOX_URL` | e2b API base |
| `CLAW_E2B_API_KEY` / `ALIYUN_E2B_TOKEN` | e2b authentication |

Project content lives in **PostgreSQL `project_config`**, not `.env`. See `docs/project-config-model.md`.

**LLM** lives in **PostgreSQL** (Admin → LLM). Gateway writes runtime files only:
- `.claw/claw-llm-runtime.env`
- `.claw/claw-tap-upstream.json`

e2b workers receive filtered env via e2b exec (`WORKER_ENV_KEYS` in `worker_env.rs`).

### Usually defaulted by profile

| Variable | `local` default | `production` default |
| --- | --- | --- |
| `CLAW_CONTAINER_RUNTIME` | `podman`（**local 强制**，`.env` 里写 auto/docker 会被覆盖） | `docker` |
| `CLAW_INTERACTIVE_BACKEND` | `e2b` | `e2b` |
| `CLAW_SOLVE_ISOLATION` | `e2b` | `e2b` |
| `GATEWAY_IMAGE` | `claw-gateway-rs:local` | *(unset — use `--release`)* |
| `GATEWAY_HOST_PORT` | `18088` | `8088` |
| `CLAW_LLM_PROXY` | `local` | `remote` + `CLAW_TAP_PROXY_URL` |
| `CLAW_IMAGE_REGISTRY` | — | `acr` (or `ghcr`) |

### e2b templates (override in `.env`)

| Variable | Purpose |
| --- | --- |
| `CLAW_E2B_WORKER_STRICT_TEMPLATE` | strict solve worker template id |
| `CLAW_E2B_WORKER_RELAXED_TEMPLATE` | relaxed worker (needs `CLAW_ALLOW_RELAXED_WORKER=1`) |
| `CLAW_NAS_HOST_MOUNT` / `CLAW_NAS_*` | NAS layout — see `docs/e2b-nas-workspace.md` |
| `CLAW_OVS_BACKEND` | `e2b` for OVS singleton |

### Optional tuning

| Variable | Purpose |
| --- | --- |
| `CLAW_ALLOW_RELAXED_WORKER` | Enable relaxed e2b worker template |
| `CLAW_MCP_MAX_CONCURRENT` | Worker MCP parallelism |
| `PLAYGROUND_ADMIN_USER` / `PLAYGROUND_ADMIN_PASSWORD` | `/admin` login |
| `CLAW_IMAGE_PREFIX` / `CLAW_IMAGE_REGISTRY` | Release image namespace |

### Deprecated (no consumers — safe to remove from `.env`)

| Variable | Was |
| --- | --- |
| `CLAW_SANDBOX_URL` / `CLAW_POOL_HTTP_*` | host pool `:9944` |
| `CLAW_SOLVE_ISOLATION=podman_pool` / `docker_pool` | local container pool |
| `CLAW_PODMAN_*_POOL_SIZE` / `CLAW_DOCKER_*` | pool daemon sizing |
| `CLAW_POOL_HOST_DAEMON` | launchd/systemd pool |

### Do not set in `.env`

| Variable | Why |
| --- | --- |
| `deploy/stack/.env` | **Forbidden** — compose implicit load breaks release pins |
| `PODMAN_HOST_SOCK` | **Removed** |

---

## Generated under `deploy/stack/` (never hand-edit)

See `docs/env-files.md`. Key paths:

- `.claw-image-release.env` — `gateway.sh up --release …`
- `.claw-build-stamp.env` — last build metadata

---

## Verify after deploy

```bash
./deploy/stack/gateway.sh verify
curl -fsS "http://127.0.0.1:${GATEWAY_HOST_PORT:-8088}/healthz"
```

---

## See also

- `docs/README.md` — documentation index
- `docs/env-files.md` — human vs generated file paths
- `deploy/stack/README.md` — operations handbook
- `docs/architecture-governance.md` — e2b topology

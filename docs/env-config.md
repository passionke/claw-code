# Environment configuration (inventory + two profiles)

Author: kejiqing

## Two supported profiles

Set **one** knob in repo root `.env`:

| `CLAW_DEPLOY_PROFILE` | OS / runtime | Solve pool | Images | Start |
| --- | --- | --- | --- | --- |
| **`local`** (default on macOS) | Podman + macOS | `podman_pool` | `pack-deploy local` → `:local` tags | `./deploy/stack/gateway.sh pack-deploy local` then `up` |
| **`production`** (default on Linux) | Docker + Linux | `docker_pool` | CI only: `up --release release-vX.Y.Z` | `./deploy/stack/gateway.sh up --release release-v1.4.5` |

Scripts apply defaults via `deploy/stack/lib/env-profile.sh` after sourcing `.env`. **Do not** mix `podman_pool` with `CLAW_DEPLOY_PROFILE=production`, or `docker_pool` with `local`.

Copy-paste starters:

- `deploy/stack/env.local.example` → merge into `.env`
- `deploy/stack/env.production.example` → merge into `.env`

Legacy snippets `env.production.docker.example` / `env.production.rootless.example` are deprecated; use the profile files above.

---

## Human-maintained (repo root `.env`)

### Required everywhere

| Variable | Purpose |
| --- | --- |
| `CLAW_DEPLOY_PROFILE` | `local` or `production` (optional: auto from OS) |
| `CLAW_GATEWAY_DATABASE_URL` | Compose default uses in-stack `postgres`; cluster uses shared external PG |

Project content (rules, skills, `CLAUDE.md`, MCP, tools) lives in **PostgreSQL `project_config`**, not `.env`. See `docs/project-config-model.md`.

**LLM (API key, upstream URL, model name)** lives in **PostgreSQL** (Admin → LLM). On startup and poll, each gateway writes **only** to generated files (never mutates human `.env`):

- `.claw/claw-llm-runtime.env` — `OPENAI_API_KEY`, `UPSTREAM_OPENAI_BASE_URL`, model names  
- `.claw/claw-tap-upstream.json` — claude-tap hot-reload target  

Pool workers read `deploy/stack/.claw-worker-runtime.env` (PG LLM file + a few deploy tunables + tap wiring). **Not** the full human `.env` — no `CLAW_GATEWAY_DATABASE_URL`. Process env is further filtered by `WORKER_ENV_KEYS` in `worker_env.rs`. Multiple gateways on the same PG share the same logical LLM config.

Prerequisite: configure at least one **active LLM** in Admin before solve.

### Usually defaulted by profile (override only if needed)

| Variable | `local` default | `production` default |
| --- | --- | --- |
| `CLAW_CONTAINER_RUNTIME` | `podman` | `docker` |
| `CLAW_SOLVE_ISOLATION` | `podman_pool` | `docker_pool` |
| `GATEWAY_IMAGE` | `claw-gateway-rs:local` | *(unset — use `--release`)* |
| `CLAW_PODMAN_IMAGE` / `CLAW_DOCKER_IMAGE` | worker `:local` / from release pin | from release pin |
| `GATEWAY_HOST_PORT` | `18088` | `8088` |
| `CLAW_LLM_PROXY` | `local` (optional sidecar `tap-up`) | `remote` + `CLAW_TAP_PROXY_URL` (shared tap) |
| `CLAW_TAP_PROXY_URL` | — | Shared claude-tap base when `CLAW_LLM_PROXY=remote` |
| `CLAW_CLUSTER_ID` | **Required** in repo root `.env` |
| Admin `clawTap` + LLM | **Required** before solve; worker `OPENAI_BASE_URL` = clawTap only |
| `CLAW_IMAGE_REGISTRY` | — | `acr` (or `ghcr`) |
| `CONTAINER_BASE_REGISTRY` | `docker.1ms.run` | — |

### Optional secrets / tuning

| Variable | Purpose |
| --- | --- |
| Per-ds `gitSyncJson` in PG | `gitUrl` / `gitRef` / PAT (`gitPatId` or inline token); not root `.env` |
| `PLAYGROUND_ADMIN_USER` / `PLAYGROUND_ADMIN_PASSWORD` | `/admin` login |
| `CLAW_GATEWAY_DATABASE_URL` | External PG only; compose default is `postgres:5432` inside stack |
| `CLAW_GATEWAY_PG_*` | Compose postgres image/credentials/host port |
| `CLAW_IMAGE_PREFIX` / `CLAW_IMAGE_REGISTRY` | Release image namespace (`gateway.sh up --release`) |
| `CLAW_POOL_ID` / `CLAW_POOL_ADVERTISE_HOST` | Pool registry override (else auto hostname/LAN IP) |
| `CLAW_WORKER_UID` / `CLAW_WORKER_GID` | Workspace ownership (default `1000:1000`); pool exec defaults to `uid:gid` when `CLAW_*_POOL_EXEC_USER` unset |
| `CLAW_SECURITY_BOOST` | Worker `run` hardening (default on); set `false` only for debug |
| `CLAW_DOCKER_POOL_EXEC_USER` / `CLAW_PODMAN_POOL_EXEC_USER` | Optional named exec/pkill user (must match passwd in image) |
| `CLAW_MCP_MAX_CONCURRENT` | Worker MCP parallelism |
| `CLAW_DEFAULT_MODEL` | Override model |
| `CLAUDE_TAP_IMAGE` | Production tap container |
| `DEEPSEEK_API_KEY` / `REPORT_LLM_PROVIDER` | Optional biz-report LLM branch |

### Do not set in `.env` (generated or wrong layer)

| Variable | Why |
| --- | --- |
| `CLAW_POOL_DAEMON_TCP` / `CLAW_POOL_HTTP_BASE` | Written to `deploy/stack/.claw-pool-rpc/gateway.env` on `up` |
| `CLAW_POOL_WORK_ROOT_HOST` | Generated `deploy/stack/.claw-pool-workspace.env` (`/var/lib/claw/workspace` in container) |
| `OPENAI_BASE_URL` | Generated `deploy/stack/.claw-worker-llm.env` (claude-tap) |
| `CLAW_CONTAINER_SOCKET` | Auto-resolved (macOS podman machine / Linux rootless); production Docker: never set |
| `PODMAN_HOST_SOCK` | **Removed** — `up` fails if present |
| `deploy/stack/.env` | **Forbidden** — compose implicit load breaks release pins |

---

## Generated under `deploy/stack/` (never hand-edit)

See `docs/env-files.md`. Key paths:

- `.claw-pool-rpc/gateway.env` — `CLAW_POOL_DAEMON_TCP`, `CLAW_POOL_HTTP_BASE`, `CLAW_POOL_RPC_HOST_WORK_ROOT`
- `.claw-pool-rpc/pool-registry.env` — `CLAW_POOL_ID`, `CLAW_POOL_ADVERTISE_HOST`
- `.claw-worker-llm.env` — tap proxy URL + pool `EXTRA_ARGS`
- `.claw-image-release.env` — `gateway.sh up --release …`

---

## Rust / worker env (code-defined)

Gateway reads many vars in `rust/crates/http-gateway-rs/src/main.rs`. Pool daemon: `src/bin/claw-pool-daemon.rs`. Docker/podman pool: `CLAW_PODMAN_*` / `CLAW_DOCKER_*` prefixes in `src/pool/docker_pool.rs`.

Worker container keys loaded from mounted `.env` (subset): `rust/crates/gateway-solve-turn/src/worker_env.rs` (`WORKER_ENV_KEYS`).

---

## 1.4.x incident map (your four symptoms)

| Symptom | Typical mis-config | Fix with profiles |
| --- | --- | --- |
| poolManager socket error | Wrong `CLAW_CONTAINER_SOCKET` or VM socket path in `.env` | Leave socket unset; `local` uses podman machine auto-detect |
| Worker not created | `docker_pool` but image lacks `docker` CLI, or `CLAW_DOCKER_IMAGE` still `:local` on server | `production` + `up --release`; pull worker image |
| Pool not registered | Host daemon cannot reach PG (`@postgres:5432` without rewrite) | Scripts rewrite to `127.0.0.1:${CLAW_GATEWAY_PG_HOST_PORT}`; ensure postgres up |
| “Mode fallback” confusion | Mixed `CLAW_PODMAN_*` + `docker_pool`, or missing `CLAW_POOL_DAEMON_TCP` in container | Gateway **requires** RPC TCP (no in-process pool); run `gateway.sh up` to regenerate `gateway.env` |

Verify after deploy:

```bash
./deploy/stack/gateway.sh verify
curl -fsS "http://127.0.0.1:${GATEWAY_HOST_PORT:-8088}/healthz"
```

---

## See also

- `docs/env-files.md` — human vs generated file paths
- `deploy/stack/README.md` — §1 稳定路径、§3 常见问题

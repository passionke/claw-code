# Environment files — who edits what

Author: kejiqing

## Rule (one human surface)

| Class | Path | Who edits |
| --- | --- | --- |
| **Human** | Repo root `.env` (from `.env.example`) | You, once per machine / env — **deploy only** (profile, ports, image registry). **No** LLM secrets |
| **Generated (PG → disk)** | `.claw/claw-llm-runtime.env`, `.claw/claw-tap-upstream.json` | **Gateway** LLM sync from PostgreSQL — **do not hand-edit** |
| **Generated** | `deploy/stack/**/*.env` except `deploy/stack/.env.example` | **`./deploy/stack/gateway.sh`** and `deploy/stack/lib/*.sh` only — **do not hand-edit** |
| **Forbidden** | `deploy/stack/.env` | **Do not create.** Compose auto-loads a `.env` next to `podman-compose.yml` and overrides root `.env` / `--release` pins. `gateway.sh up` **fails** if this file exists. |

Runtime JSON beside env (gateway / tap): `.claw/claw-tap-upstream.json` — written by the gateway from PostgreSQL (Admin LLM), not a second “component `.env`”.

## Generated files (copy / overwrite = deploy scripts only)

Each file is recreated when you run the matching flow (`gateway.sh up`, `tap-up`, release pin, etc.). First line is usually `# GENERATED — do not edit`.

| File | Typical writer |
| --- | --- |
| `deploy/stack/.claw-worker-llm.env` | `lib/worker-llm-wiring.sh` (via `up.sh` / tap) |
| `deploy/stack/.claw-worker-runtime.env` | `lib/worker-llm-wiring.sh` |
| `deploy/stack/.claw-llm-runtime.env` | `lib/compose-include.sh` → `claw_export_llm_runtime_layout` |
| `deploy/stack/.claw-pool-daemon.env` | `lib/compose-include.sh` → `claw_podman_write_pool_daemon_sidecar_env` |
| `deploy/stack/.claw-pool-workspace.env` | `lib/compose-include.sh` → `claw_podman_export_pool_workspace` |
| `deploy/stack/.claw-pool-rpc/gateway.env` | `lib/compose-include.sh` (pool TCP/HTTP for gateway) |
| `deploy/stack/.claw-pool-rpc/pool-registry.env` | `lib/claw-pool-registry-env.sh` |
| `deploy/stack/.claw-image-release.env` | `lib/release-images.sh` (`up --release …`) |
| `deploy/stack/.claw-pool-worker.env` | `lib/release-images.sh` (worker image pin) |
| `deploy/stack/.claw-build-stamp.env` | `lib/claw-write-build-stamp.sh` |

If you need a new key for containers or workers, add it to **repo root `.env`** (or the code that generates the snippet), then re-run **`./deploy/stack/gateway.sh up`** — not a new hand-maintained file under `web/` or `deploy/stack/`.

## Deploy profiles

Set `CLAW_DEPLOY_PROFILE=local|production` in repo root `.env`. Defaults: macOS → `local`, Linux → `production`. Full variable inventory: **`docs/env-config.md`**.

Starters: `deploy/stack/env.local.example`, `deploy/stack/env.production.example`.

## See also

- `docs/env-config.md` — 全量 env 表 + 双模式对照 + 1.4.x 排障
- `deploy/stack/README.md` — §6 环境变量
- `.env.example` — 最小模板

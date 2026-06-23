# Self-hosted e2b (interactive OVS / terminal)

Author: kejiqing

Interactive sessions (`terminal/start`, `agent/ws`, `ovs-*`) run on **self-hosted e2bserver** (E2B-compatible API) with NAS bind mounts. **solve_async** stays on the local podman worker pool — unchanged.

**NAS layout:** [`docs/fc-nas-workspace.md`](../../docs/fc-nas-workspace.md)  
**Env template:** `deploy/stack/env.selfhosted-e2b.example`

## Single source of truth for `claw` / `ttyd`

Binaries live **only** in the **e2b template image** (`/usr/local/bin/claw`, `/usr/local/bin/ttyd`). There is no NAS copy or runtime bootstrap.

After changing `claw` or `ttyd`:

```bash
./deploy/stack/gateway.sh pack-deploy          # rebuild claw-gateway-worker image
python3 deploy/fc-sandbox/build-claw-worker-selfhosted.py   # rebuild claw-worker template
./deploy/stack/gateway.sh pool-reset
./deploy/stack/gateway.sh up
```

`build-claw-worker-selfhosted.py` extracts `claw` + `ttyd` from `CLAW_FC_WORKER_IMAGE`, uploads build context via e2b API (`COPY` in Dockerfile — no HTTP artifact server).

## Prerequisites

1. Self-hosted e2bserver (e.g. `10.8.0.9`) + API key
2. NFS export mounted on e2b host (`hostMountRoot` for `nasConfig` binds)
3. Gateway compose NFS volume or host bind to the same export tree
4. Templates built: `claw-worker`, `claw-ovs`, `claw-observe`

One-shot ECS setup: `deploy/stack/lib/setup-selfhosted-e2b.sh` (mount NAS → build template → `gateway.sh up`).

## NAS bind (gateway → e2b)

Gateway creates sandboxes with **`nasConfig`** (`hostMountRoot` + `relPath` → `mountDir`):

| relPath | guest |
|---------|-------|
| `proj_N/home` | `/claw_ds` |
| `proj_N/workers/{workerId}` | `/claw_host_root` |
| `` (export root) | `/claw_ws` |

Gateway mkdir/symlink on `CLAW_NAS_HOST_MOUNT`; e2b only host-binds. No in-sandbox NFS mount.

## `fc_exec.py`

Python helper: stdin JSON → envd `commands.run` inside sandbox. Used for OVS agent scripts and `gateway-solve-once` in FC solve path. Bundled in gateway image as `CLAW_FC_EXEC_HELPER`.

## E2E verify

```bash
./deploy/stack/lib/verify-fc-ovs-e2e.sh
./deploy/stack/lib/verify-ovs-claw-e2e.sh   # optional multi-turn when CLAW_NAS_HOST_MOUNT set
```

## Key env

See `deploy/stack/env.selfhosted-e2b.example`:

| Variable | Role |
|----------|------|
| `CLAW_INTERACTIVE_BACKEND=fc` | Interactive on e2b instead of podman pool |
| `CLAW_FC_API_URL` / `CLAW_FC_DOMAIN` | Self-hosted e2b API |
| `CLAW_FC_TEMPLATE` | Worker template name (default `claw-worker`) |
| `CLAW_FC_WORKER_IMAGE` | Source image for template build (set by pack-deploy) |
| `CLAW_NAS_HOST_MOUNT` | Host path to NFS export (gateway + e2b bind root) |
| `CLAW_FC_EXEC_HELPER` | Default `deploy/fc-sandbox/fc_exec.py` |

## Rust client

`rust/crates/claw-fc-sandbox-client/` — E2B REST + Python envd exec helper.

Interactive routing: `http-gateway-rs` → `InteractiveSandboxBackend` (`podman` | `fc`).

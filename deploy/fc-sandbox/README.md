# Alibaba FC cloud sandbox (interactive OVS / terminal)

Author: kejiqing

Interactive sessions (`terminal/start`, `agent/ws`, `ovs-*`) can run on **Alibaba Cloud FC sandbox** (E2B-compatible API, **cn-beijing only**) instead of the local podman worker pool. **solve_async** stays on `claw-sandbox` — unchanged.

## Cost (NAS)

| Item | Value |
| --- | --- |
| NAS unit price | ¥0.001 / GB / hour |
| Planned capacity | 100 GB |
| Approx. yearly | **¥876 / year** |

FC sandbox runtime is billed separately (MicroVM uptime; use sleep/wake to reduce cost).

## Prerequisites

1. FC cloud sandbox enabled in **华北2 北京** + SLR + API Key (`e2b_…`)
2. NAS file system in **same region** (cn-beijing)
3. For FC dynamic NAS mount: NAS VPC mount point + security group **2049/TCP**
4. Gateway / OVS: compose **NFS volume** mounts NAS inside containers (no Mac host mount); or run stack on Beijing ECS in VPC

## Phase 0 — verify before gateway code path

### Step A — FC API (no NAS)

From repo root (`.env` with `ALIYUN_E2B_TOKEN` or `CLAW_FC_API_KEY`):

```bash
set -a && source .env && set +a
export E2B_API_KEY="${CLAW_FC_API_KEY:-${ALIYUN_E2B_TOKEN}}"
export E2B_DOMAIN="${CLAW_FC_DOMAIN:-cn-beijing.e2b.fc.aliyuncs.com}"

python3 -m venv /tmp/fc-quickstart-venv
source /tmp/fc-quickstart-venv/bin/activate
pip install e2b-code-interpreter -q
python3 deploy/fc-sandbox/quickstart.py
```

Pass: prints `hello from fc` and a `sandbox_id`.

### Step B — Gateway + OVS 直挂 NAS（无需 Mac 宿主机 mount）

在 repo 根 `.env`：

```bash
NAS_BASE_URL=xxx.cn-beijing.nas.aliyuncs.com
CLAW_FC_NAS_EXPORT=/claw-workspace
CLAW_USE_NAS_VOLUME=auto   # NAS_BASE_URL 已设时默认开启；=0 退回本地 bind
```

`./deploy/stack/gateway.sh up` 生成 compose NFS volume（`deploy/stack/.claw-workspace-volume.yml`），**Podman 在 Gateway/OVS 容器内直接挂 NAS**。

验收：

```bash
./deploy/stack/gateway.sh up
podman exec claw-gateway-rs sh -c 'echo ok > /var/lib/claw/workspace/.probe'
podman exec claw-openvscode-server ls -la /home/workspace/.probe
```

**solve podman pool** 仍用本机 `deploy/stack/claw-workspace` 作 worker bind（与 Gateway/OVS 的 NAS 树分离，直到 solve 迁远程 pool）。

### Step C — custom template + NAS dynamic mount

1. `./deploy/fc-sandbox/build-template.sh` — build `claw-gateway-worker` image
2. Publish FC template **`claw-worker-v1`** (console; image must include `ttyd` + `claw`)
3. Configure template VPC + NAS volume name → set `CLAW_FC_NAS_VOLUME_NAME` in `.env`
4. Full stack: copy `deploy/stack/env.fc-interactive.example` → merge into `.env`, set `CLAW_INTERACTIVE_BACKEND=fc`

## Gateway env (interactive FC mode)

See `deploy/stack/env.fc-interactive.example`. Key variables:

| Variable | Role |
| --- | --- |
| `CLAW_INTERACTIVE_BACKEND=fc` | Use FC instead of podman pool for interactive |
| `CLAW_FC_API_KEY` | FC / E2B API key (fallback: `ALIYUN_E2B_TOKEN`) |
| `CLAW_FC_API_URL` | Default `https://api.cn-beijing.e2b.fc.aliyuncs.com` |
| `CLAW_FC_DOMAIN` | Default `cn-beijing.e2b.fc.aliyuncs.com` |
| `CLAW_FC_TEMPLATE` | e.g. `claw-worker-v1` |
| `CLAW_USE_NAS_VOLUME` | `auto`（有 `NAS_BASE_URL` 即 compose NFS 直挂） |
| `CLAW_FC_NAS_EXPORT` | NAS export 子路径，默认 `/claw-workspace` |
| `CLAW_FC_NAS_VOLUME_NAME` | FC template NAS volume for dynamic mount |
| `CLAW_FC_EXEC_HELPER` | Default `deploy/fc-sandbox/fc_exec.py` (needs `e2b-code-interpreter` on gateway host) |

## Rust client

`rust/crates/claw-fc-sandbox-client/` — minimal E2B REST (`POST /sandboxes`, `DELETE /sandboxes/{id}`) + Python envd exec helper.

Interactive routing: `http-gateway-rs` → `InteractiveSandboxBackend` (`podman` | `fc`).

## E2E verify

```bash
./deploy/stack/lib/verify-fc-ovs-e2e.sh
```

Requires `CLAW_INTERACTIVE_BACKEND=fc`, gateway up, and (for full OVS chat) LLM configured.

## References

- [FC sandbox overview](https://help.aliyun.com/zh/functioncompute/fc/what-is-a-fc-sandbox)
- [SDK quickstart](https://help.aliyun.com/zh/functioncompute/fc/create-your-first-cloud-sandbox-via-the-sdk)
- [Dynamic NAS mount](https://help.aliyun.com/zh/functioncompute/fc/user-guide/dynamically-mount-a-file-storage-nas)
- Repo plan: `docs/boundaries-claw-stack.md` (FC interactive section)

# FC NAS 工作空间 — 唯一逻辑根与各组件本地视图

Author: kejiqing

**目的：** 一份文档说清「同一份 NFS  export 根」在 Gateway、e2b、Podman 容器里各自长什么样、谁 mkdir、谁 bind。**避免把 host 路径字符串与容器 bind 点混为一谈。**

**Invariant（NAS 进 sandbox）：** 仅 **host 直 bind**（`{hostMountRoot}/{relPath}` → guest `mountDir`）。**禁止** rsync / `data/nas-bind` 副本 — 该路径已从 e2bserver 移除（2026-06）。

相关：[`deploy/stack/env.selfhosted-e2b.example`](../deploy/stack/env.selfhosted-e2b.example)、[`claw-fc-sandbox-client/src/nas_paths.rs`](../rust/crates/claw-fc-sandbox-client/src/nas_paths.rs)、[`http-gateway-rs/src/pool/fc_nas_layout.rs`](../rust/crates/http-gateway-rs/src/pool/fc_nas_layout.rs)、[`docs/ovs-chat/FC-OVS-SINGLETON-DESIGN.md`](ovs-chat/FC-OVS-SINGLETON-DESIGN.md)。

---

## 1. 唯一逻辑根（SoT）

所有 FC 交互模式（OVS singleton、worker warm pool、fc-cloud solve）共享 **同一个 NAS export 树**，用 **相对 export 根的逻辑路径** 描述，与具体机器挂载点无关：

```text
<export-root>/                    ← 逻辑根（relPath ""）
├── proj_{N}/home/                ← 项目 home（PG materialize、OVS folder）
├── proj_{N}/workers/{workerId}/  ← worker 工作区（e2b bind → /claw_host_root）
├── proj_{N}/sessions/{segment}/  ← 必须是 symlink → ../workers/{workerId}
├── tap-traces/                   ← claude-tap traces（可选）
└── .claw-pool-work/              ← 本机 podman_pool solve 用（与 FC worker 树分离）
```

**Invariant：**

- Gateway **唯一**负责在 NAS 上 `mkdir` / session symlink（`fc_nas_layout`）。
- e2b **只**按 `nasConfig` 做 `{hostMountRoot}/{relPath}` → guest `mountDir` bind，**不**在 sandbox 内 `mount.nfs4`。
- `sessions/{segment}` 在 FC NAS 模式下 **必须是 symlink**；旧版实体目录会在 link 时被 rename 为 `.legacy-*` 再建链。

---

## 2. 各组件：本地路径 ↔ 容器 bind

同一份数据，不同进程看到的 **本机绝对路径** 不同；容器里 bind 的又是第三层路径。

| 组件 | 跑在哪 | 本机如何看到 NAS | 进程/容器内 work/bind 点 | 谁写盘 |
|------|--------|------------------|---------------------------|--------|
| **NAS 文件** | 10.8.0.8 NFS | — | export `10.8.0.8:/`（或 `/mnt/NAS0/nfs-export`） | — |
| **Gateway** | 本机 Podman 容器 | Mac host: `/Volumes/claw-nas` | 容器: `CLAW_WORK_ROOT` = `/var/lib/claw/workspace`（compose 直 bind NAS） | mkdir `workers/`、symlink `sessions/`、materialize `home/` |
| **e2b（Mac / Linux）** | 10.8.0.9 等 | `/Volumes/claw-nas` 或 `/mnt/nas0` | FC 沙箱 guest: `/claw_ws` / `/claw_ds` / `/claw_host_root` | 仅直 bind `{hostMountRoot}/{relPath}`；写盘在 guest 内落到 NAS |
| **OVS compose**（迁移期） | Gateway 同栈 | 同 Gateway | `/home/workspace` | 非 FC 时用 compose volume |
| **podman_pool worker** | Gateway host | `CLAW_POOL_WORK_ROOT_BIND_SRC`（可与 NAS 同盘不同目录） | worker 容器 `/claw_ds` 等 | pool daemon |

Admin 只读镜像：`GET /v1/gateway/global-settings` → `fcNas`（`nasHostMount`、`nasRootResolved`、`layoutActive`）。**改 NAS 只改 repo 根 `.env`，重启 gateway。**

---

## 3. 逻辑路径 → guest 挂载（e2b `nasConfig`）

代码单一来源：`rust/crates/claw-fc-sandbox-client/src/nas_paths.rs`。

| 场景 | relPath（相对 export 根） | guest mountDir |
|------|---------------------------|----------------|
| OVS / observe singleton | ``（空 = export 根） | `/claw_ws` |
| Worker warm / fc-cloud solve | `proj_N/home` | `/claw_ds` |
| Worker warm / fc-cloud solve | `proj_N/workers/{workerId}` | `/claw_host_root` |

create sandbox 时 Gateway 发送：

```json
{
  "hostMountRoot": "/Volumes/claw-nas",
  "mountPoints": [
    { "relPath": "proj_3/workers/wrk_abc", "mountDir": "/claw_host_root" },
    { "relPath": "proj_3/home", "mountDir": "/claw_ds" }
  ]
}
```

e2b 解析为 host 绝对路径 `{hostMountRoot}/{relPath}`，再 bind 进 FC 微VM。

---

## 4. Gateway：两个 env、一个写盘根

| 变量 | 含义 |
|------|------|
| `CLAW_NAS_HOST_MOUNT` | **e2b 所在 Mac/Linux host** 上 NFS 挂载点（如 `/Volumes/claw-nas`、`/mnt/nas0`）。写入 `nasConfig.hostMountRoot`，**不是** Gateway 容器内路径。 |
| `CLAW_WORK_ROOT` | Gateway **容器内** workspace 根（默认 `/var/lib/claw/workspace`）。compose 把 NAS **直 bind** 到这里。 |
| **实际 mkdir 根** | `nas_host_root(work_root)`：若容器内不存在 `CLAW_NAS_HOST_MOUNT`，则用 `work_root`（Mac dev 常态）。 |

```text
Mac dev（Gateway 在 Podman 里）：

  Host NFS mount     /Volumes/claw-nas/proj_3/workers/wrk_x
         │ compose bind（无 :U）
         ▼
  Gateway 容器       /var/lib/claw/workspace/proj_3/workers/wrk_x   ← fc_nas_layout 写这里

  e2b host           /Volumes/claw-nas/proj_3/workers/wrk_x         ← nasConfig 指向这里
         │ podman -v（Mac 见 §5）
         ▼
  FC sandbox guest   /claw_host_root/...
```

**常见坑：** 容器内没有 `/Volumes/claw-nas`，但 `CLAW_NAS_HOST_MOUNT` 仍应设置（给 e2b）；Gateway 写盘只看 `nasRootResolved` = work_root。

---

## 5. Mac + Podman：直 bind（唯一路径）

Podman Desktop / podman machine 跑在 Linux VM 里。**VM 默认看不见** Mac host 的 `/Volumes/claw-nas`，必须在 machine init 显式挂载；挂好后 e2b 与 Linux 相同，**直 bind** `{hostMountRoot}/{relPath}`。

**Gateway 本机** 与 **e2b 所在 Mac（10.8.0.9）** 均需：

```bash
podman machine stop
podman machine rm -f   # 无 NAS 挂载的旧 machine 需重建
podman machine init \
  -v /Volumes/claw-nas:/Volumes/claw-nas \
  -v /Users/<you>/work/claw-code:/Users/<you>/work/claw-code   # Gateway dev 还要 repo
podman machine set --memory 16384   # 本地编译可按需
podman machine start
```

e2b create：`podman -v /Volumes/claw-nas/proj_N/workers/wrk_x:/claw_host_root`（guest 内为 virtiofs bind）。

```text
Gateway mkdir  →  /Volumes/claw-nas/proj_N/workers/wrk_x
       │
       │  POST /sandboxes + nasConfig（同一 host 路径）
       ▼
e2b podman -v  →  guest /claw_host_root
```

**勿再使用（已废弃）：** `sync-nas-bind*.sh`、`data/nas-bind/root` rsync 副本。若 create 仍提示 `run sync-nas-bind`，说明 e2bserver 版本过旧，需升级并确认 `runtime_bind_source` 走 `host_canonical`。

---

## 6. 环境变量（self-hosted 10.8.0.x）

合并模板：[`deploy/stack/env.selfhosted-e2b.example`](../deploy/stack/env.selfhosted-e2b.example)。

| 变量 | Mac dev 示例 | Linux ECS 示例 |
|------|--------------|----------------|
| `CLAW_NAS_HOST_MOUNT` | `/Volumes/claw-nas` | `/mnt/nas0` |
| `CLAW_FC_NAS_SERVER` | `10.8.0.8` | 同左 |
| `CLAW_FC_NAS_EXPORT` | `/` | `/` |
| `CLAW_USE_NAS_VOLUME` | `0`（compose 直 bind host mount） | `0` 或 compose NFS volume |
| `CLAW_POOL_WORK_ROOT_BIND_SRC` | `/Volumes/claw-nas/.claw-pool-work` | `/mnt/nas0/.claw-pool-work` |
| `CLAW_TAP_TRACES_DIR` | `/Volumes/claw-nas/tap-traces` | `/mnt/nas0/tap-traces` |

Gateway compose NAS bind **不要** `:U`（NFS 不能 chown）：见 `deploy/stack/lib/compose-include.sh` → `CLAW_GATEWAY_WORKSPACE_BIND`。

---

## 7. 端到端时序（fc-cloud solve）

```text
1. Gateway acquire_slot
      → ensure_worker_root_on_nas(work_root, proj, wrk_x)   # NAS 上 mkdir
      → link_session_to_worker → sessions/{id} → ../workers/wrk_x

2. Gateway POST e2b /sandboxes + nasConfig(relPath=proj_N/workers/wrk_x)

3. e2b host
      → 解析 {hostMountRoot}/proj_N/workers/wrk_x 存在
      → podman 直 bind 该路径
      → FC guest 看到 /claw_host_root（virtiofs）

4. Gateway materialize / exec solve
      → 写经 work_root 或 guest bind 落到同一 NAS 文件
```

---

## 8. 验收（必须有证据再称「通」）

```bash
# NAS 在 host 已挂
mount | grep claw-nas    # Mac
# 或 mountpoint /mnt/nas0

# Gateway 布局
curl -s http://127.0.0.1:8088/v1/gateway/global-settings | jq .fcNas
# layoutActive: true, nasRootResolved: /var/lib/claw/workspace

# e2b NAS 就绪
curl -s http://10.8.0.9:3000/health | jq .nas
# ready: true, hostMountRoot: "/Volumes/claw-nas", sandboxInject: "bind"

# 写盘探针
podman exec claw-gateway-rs sh -c 'mkdir -p /var/lib/claw/workspace/.probe && echo ok'
ls /Volumes/claw-nas/.probe

# e2b nasConfig bind（worker 三挂载点）
./deploy/stack/lib/verify-e2b-nas-inject.sh

# FC 全链路（含 terminal / OVS）
CLAW_FC_E2E_CLEANUP=0 ./deploy/stack/lib/verify-fc-ovs-e2e.sh

# fc-cloud solve（proj sandbox 模式）
# POST /v1/solve_async → poll /v1/tasks/{sessionId} → status succeeded
```

---

## 9. 故障速查

| 现象 | 常见根因 | 检查 |
|------|----------|------|
| gateway-rs 起不来 `lchown … :U` | compose 对 NFS bind 用了 `:U` | `CLAW_GATEWAY_WORKSPACE_BIND` 无 `:U` |
| `mirror CLAUDE.md … Operation not permitted` | NFS 上 `fs::copy` 失败 | 应用 `write` 双份，勿 `copy` |
| `NAS host path not found` | Gateway 未 mkdir | `fcNas.layoutActive`；`ls …/proj_N/workers/` |
| `podman bind path missing` / `sync-nas-bind` | host 路径不存在，或 podman machine 未 `-v` NAS，或 e2b 未升级 | §5；`verify-e2b-nas-inject.sh` |
| `Directory not empty` on session link | 旧 session 实体目录 | 已 rename `.legacy-*`；勿在 FC 模式直接写 `sessions/` 目录 |
| playground `Name or service not known` | gateway-rs 未运行 | `podman ps claw-gateway-rs` |

详细踩坑记录：[`docs/ovs-chat/FC-OVS-E2E-FAILURES.md`](ovs-chat/FC-OVS-E2E-FAILURES.md)。

---

## 10. 代码索引

| 职责 | 路径 |
|------|------|
| 逻辑 relPath / guest 常量 | `rust/crates/claw-fc-sandbox-client/src/nas_paths.rs` |
| Gateway mkdir / symlink | `rust/crates/http-gateway-rs/src/pool/fc_nas_layout.rs` |
| fc-cloud pool acquire | `rust/crates/http-gateway-rs/src/pool/fc_orchestrated_pool.rs` |
| warm pool | `rust/crates/http-gateway-rs/src/pool/interactive_backend/fc_warm_pool.rs` |
| nasConfig JSON | `rust/crates/claw-fc-sandbox-client/src/client.rs` |
| compose NAS bind | `deploy/stack/lib/compose-include.sh` |
| Admin 只读 fcNas | `rust/crates/http-gateway-rs/src/gateway_fc_nas_settings.rs` |
| e2b host 直 bind | `e2bserver/crates/e2b-core/src/nas.rs`（独立仓库） |
| e2b bind 验收 | `deploy/stack/lib/verify-e2b-nas-inject.sh` |

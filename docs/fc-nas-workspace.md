# FC NAS 工作空间 — 唯一逻辑根与各组件本地视图

Author: kejiqing

**目的：** 一份文档说清「同一份 NFS  export 根」在 Gateway、e2b、Podman 容器里各自长什么样、谁 mkdir、谁 bind。**避免把 host 路径字符串与容器 bind 点混为一谈。**

**Invariant（NAS 进 sandbox）：** 仅 **host 直 bind**（`{hostMountRoot}/{relPath}` → guest `mountDir`）。**禁止** rsync / `data/nas-bind` 副本 — 该路径已从 e2bserver 移除（2026-06）。

相关：[`architecture-governance.md`](architecture-governance.md)（**canonical IP 对照表**）、[`FC-WORKER-CONTEXT-PLAN.md`](fc-nas/FC-WORKER-CONTEXT-PLAN.md)、[`deploy/stack/env.selfhosted-e2b.example`](../deploy/stack/env.selfhosted-e2b.example)、[`claw-fc-sandbox-client/src/nas_paths.rs`](../rust/crates/claw-fc-sandbox-client/src/nas_paths.rs)、[`http-gateway-rs/src/pool/fc_nas_layout_backend.rs`](../rust/crates/http-gateway-rs/src/pool/fc_nas_layout_backend.rs)。

---

## 1. 唯一逻辑根（SoT）

所有 FC 交互模式（OVS singleton、worker warm pool、fc-cloud solve）共享 **同一个 NAS export 树**，用 **相对 export 根的逻辑路径** 描述，与具体机器挂载点无关：

```text
<export-root>/                              ← 逻辑根（relPath ""）
├── {clusterId}/                              ← CLAW_CLUSTER_ID（多集群隔离）
│   └── proj_{N}/
│       ├── home/                             ← ds_home（管理后台 materialize；worker 只读 bind）
│       ├── sessions/{sessionId}/             ← 真实目录（OVS + resolve 上下文 SoT）
│       └── workers/{workerId}/               ← 执行缓存（e2b bind → /claw_host_root）
├── .claw-fc-tools/                           ← claw / ttyd / claude-tap（FC bootstrap 拷贝源）
└── tap-traces/                               ← claude-tap traces（可选）
```

**Invariant：**

- Gateway **不** bind-mount NAS；所有 NAS 写盘（`mkdir` / `put` / `symlink` / readback `get`）经 **claw-nas-api** e2b singleton HTTP（`fc_nas_layout_backend` + `FcNasApiSingleton`）。
- e2b **只**按 `nasConfig` 做 `{hostMountRoot}/{relPath}` → guest `mountDir` bind，**不**在 sandbox 内 `mount.nfs4`。
- `sessions/{sessionId}` 为**真实目录**；禁止再把 session 目录 symlink 到 worker 目录。
- `home/` 仅管理后台可写；worker 侧 `/claw_ds` 为只读 bind。

---

## 2. 各组件：本地路径 ↔ 容器 bind

同一份数据，不同进程看到的 **本机绝对路径** 不同；容器里 bind 的又是第三层路径。

| 组件 | 跑在哪 | 本机如何看到 NAS | 进程/容器内 work/bind 点 | 谁写盘 |
|------|--------|------------------|---------------------------|--------|
| **NAS 文件** | `10.8.0.11` NFS | — | export `10.8.0.11:/` | — |
| **Gateway** | 本机 Podman 容器 | **不挂 NAS**（排查时可本地 mount，非生产路径） | `CLAW_WORK_ROOT` = 本地 virtiofs | 经 **claw-nas-api** HTTP 操作 NAS |
| **e2b** | `10.8.0.1` | e2b 宿主机 `/mnt/nas0` | FC 沙箱 guest: `/claw_ws` / `/claw_ds` / `/claw_host_root` | host 直 bind `{hostMountRoot}/{relPath}` |
| **claw-nas-api** | e2b singleton | `/claw_ws` = NAS export 根 | HTTP `:8090` | mkdir / put / get / symlink |

Admin 只读镜像：`GET /v1/gateway/global-settings` → `fcNas`（`nasHostMount`、`nasRootResolved`、`layoutActive`）。**改 NAS 只改 repo 根 `.env`，重启 gateway。**

---

## 3. 逻辑路径 → guest 挂载（e2b `nasConfig`）

代码单一来源：`rust/crates/claw-fc-sandbox-client/src/nas_paths.rs`。

| 场景 | relPath（相对 export 根） | guest mountDir | 权限 |
|------|---------------------------|----------------|------|
| OVS / observe singleton | ``（空 = export 根） | `/claw_ws` | rw |
| Worker warm / solve / OVS agent | `{clusterId}/proj_N/home` | `/claw_ds` | **ro** |
| Worker warm / solve / OVS agent | `{clusterId}/proj_N/sessions` | `/claw_sessions` | rw |
| Worker warm / solve / OVS agent | `{clusterId}/proj_N/workers/{workerId}` | `/claw_host_root` | rw（缓存） |

create sandbox 时 Gateway 发送：

```json
{
  "hostMountRoot": "/Volumes/claw-nas",
  "mountPoints": [
    { "relPath": "prod-claw-01/proj_3/workers/wrk_abc", "mountDir": "/claw_host_root" },
    { "relPath": "prod-claw-01/proj_3/sessions", "mountDir": "/claw_sessions" },
    { "relPath": "prod-claw-01/proj_3/home", "mountDir": "/claw_ds", "readOnly": true }
  ]
}
```

e2b 解析为 host 绝对路径 `{hostMountRoot}/{relPath}`，再 bind 进 FC 微VM。

---

## 4. Gateway 与 NAS：经 claw-nas-api，不直连

| 变量 | 含义 |
|------|------|
| `CLAW_FC_API_URL` | e2bserver API（`10.8.0.1:3000`） |
| PG `fcNasApi.baseUrl` | claw-nas-api singleton HTTP 入口（`./deploy/stack/gateway.sh nas-api-up` 写入） |
| `CLAW_E2B_NAS_HOST_MOUNT` | **e2b 宿主机** NAS 挂载点（如 `/mnt/nas0`）→ 写入 `nasConfig.hostMountRoot` |
| `CLAW_WORK_ROOT` | Gateway 容器内本地 workspace（**不是** NAS） |

```text
Gateway 容器                claw-nas-api singleton (e2b)       NAS (10.8.0.11)
  HTTP mkdir/put/get  ──►   /claw_ws = export 根  ──►        {cluster}/proj_N/...
  不读本地盘做 readback
```

Mac 本地 `/Volumes/claw-nas` **仅排查用**；Gateway 生产路径禁止依赖该 mount。

---

## 5. Mac + Podman：直 bind（唯一路径）

Podman Desktop / podman machine 跑在 Linux VM 里。**VM 默认看不见** Mac host 的 `/Volumes/claw-nas`，必须在 machine init 显式挂载；挂好后 e2b 与 Linux 相同，**直 bind** `{hostMountRoot}/{relPath}`。

**e2b 宿主机（`10.8.0.1`）** 需挂载 NAS 到 `/mnt/nas0`（Mac dev 另机排查可挂 `/Volumes/claw-nas`，**不给 Gateway 容器 bind**）：

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
Gateway HTTP  ──►  claw-nas-api  ──►  {cluster}/proj_N/workers/wrk_x  (NAS export)
       │
       │  POST /sandboxes + nasConfig（e2b host bind 同一 relPath）
       ▼
e2b podman -v  →  guest /claw_host_root
```

**勿再使用（已废弃）：** `sync-nas-bind*.sh`、`data/nas-bind/root` rsync 副本。若 create 仍提示 `run sync-nas-bind`，说明 e2bserver 版本过旧，需升级并确认 `runtime_bind_source` 走 `host_canonical`。

---

## 6. 环境变量（self-hosted 10.8.0.x）

合并模板：[`deploy/stack/env.selfhosted-e2b.example`](../deploy/stack/env.selfhosted-e2b.example)。

| 变量 | Mac dev 示例 | Linux e2b 宿主机 |
|------|--------------|------------------|
| `CLAW_FC_API_URL` | `http://10.8.0.1:3000` | 同左 |
| PG `fcNasApi.baseUrl` | **必填**（`gateway.sh nas-api-up` 写入）；未配置则 gateway **启动失败** | 同左 |
| `CLAW_E2B_SANDBOX_URL` | `http://10.8.0.1:3002` | 同左 |
| `CLAW_FC_NAS_SERVER` | `10.8.0.11` | 同左 |
| `CLAW_E2B_NAS_HOST_MOUNT` | `/mnt/nas0`（e2b 宿主机） | `/mnt/nas0` |
| `CLAW_USE_NAS_VOLUME` | `0` | `0` |
| `CLAW_TAP_TRACES_DIR` | `/mnt/nas0/tap-traces` | 同左 |

Gateway **不设** `CLAW_NAS_HOST_MOUNT`（不直连 NAS）。Canonical IP 见 [`architecture-governance.md`](architecture-governance.md) §1。

---

## 7. 端到端时序（fc-cloud solve）

```text
1. Gateway acquire_slot
      → ensure_worker_root_on_nas(work_root, cluster, proj, wrk_x)
      → ensure_session_root_on_nas(work_root, cluster, proj, session_segment)

2. Gateway POST e2b /sandboxes + nasConfig
      relPath={cluster}/proj_N/home      → /claw_ds (ro)
      relPath={cluster}/proj_N/sessions  → /claw_sessions
      relPath={cluster}/proj_N/workers/wrk_x → /claw_host_root

3. e2b host
      → 解析 {hostMountRoot}/{relPath}
      → podman 直 bind 这些路径
      → FC guest 看到 /claw_ds、/claw_sessions、/claw_host_root

4. Gateway exec solve
      → inline 写入 /claw_sessions/{segment}/gateway-solve-task.json（仅 task_json）
      → worker 直接读写 /claw_sessions/{segment}/.claw/gateway-solve-session.jsonl（续聊 SoT，**不**从 DB 回灌）
      → solve 结束后 readback：NAS jsonl 全量 reconcile → DB `cc_messages`（单向索引）
      → **不**将 session 目录 gzip-tar 写回 DB（`gateway_session_artifacts` workspace tar 仅 legacy docker pool）
```

---

## 7b. Transcript 边界（FC solve 续聊）

| 方向 | 做不做 | 说明 |
|------|--------|------|
| NAS `gateway-solve-session.jsonl` → worker 续聊 | **做** | 唯一 SoT；runtime `load_from_path` / append |
| NAS jsonl → DB `cc_messages` | **做** | solve 结束后经 **claw-nas-api `GET /v1/files`** readback，全量 reconcile `cc_messages` |
| Gateway 本地盘读 jsonl | **不做** | Gateway 不挂 NAS；readback 只走 nas-api |
| DB `render_session_jsonl` → worker inline | **不做** | 禁止回灌覆盖 NAS 文件 |
| session 目录 gzip-tar → DB | **不做** | FC 主路径不写 `gateway_session_artifacts` workspace tar |
| DB → 覆盖 NAS jsonl | **不做** | 禁止反向写盘 |

逻辑路径：`{clusterId}/proj_{N}/sessions/{sessionId}/.claw/gateway-solve-session.jsonl`。

代码：`fc_orchestrated_pool.rs`（`session_jsonl: None`）、`fc_nas_layout_backend.rs`（`read_session_jsonl`）、`fc_nas_api_singleton.rs`（`get_file`）、`session_db_sync.rs`、`persistence/transcript.rs`。

---

## 8. 验收（必须有证据再称「通」）

```bash
# e2b 宿主机 NAS 已挂（10.8.0.1 上 /mnt/nas0；Mac 排查可挂 /Volumes/claw-nas，不给 Gateway bind）

# nas-api singleton 健康
curl -s "$(curl -s http://127.0.0.1:8088/v1/gateway/global-settings | jq -r .fcNasApi.baseUrl)/healthz"

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
| `Directory not empty` on session root | 旧 symlink/异常文件占用 session 路径 | Gateway 会移除旧 symlink 或 rename 旧实体目录为 `.legacy-*` 后创建真实 session root |
| playground `Name or service not known` | gateway-rs 未运行 | `podman ps claw-gateway-rs` |

详细踩坑记录：[`docs/ovs-chat/FC-OVS-E2E-FAILURES.md`](ovs-chat/FC-OVS-E2E-FAILURES.md)。

---

## 10. 代码索引

| 职责 | 路径 |
|------|------|
| 逻辑 relPath / guest 常量 | `rust/crates/claw-fc-sandbox-client/src/nas_paths.rs` |
| Gateway mkdir / symlink | `rust/crates/http-gateway-rs/src/pool/fc_nas_layout.rs` |
| FC solve readback / reconcile | `rust/crates/http-gateway-rs/src/pool/session_db_sync.rs` |
| fc-cloud pool acquire / solve exec | `rust/crates/http-gateway-rs/src/pool/fc_orchestrated_pool.rs` |
| jsonl ↔ `cc_messages` | `rust/crates/http-gateway-rs/src/persistence/transcript.rs` |
| warm pool | `rust/crates/http-gateway-rs/src/pool/interactive_backend/fc_warm_pool.rs` |
| nasConfig JSON | `rust/crates/claw-fc-sandbox-client/src/client.rs` |
| compose NAS bind | `deploy/stack/lib/compose-include.sh` |
| Admin 只读 fcNas | `rust/crates/http-gateway-rs/src/gateway_fc_nas_settings.rs` |
| e2b host 直 bind | `e2bserver/crates/e2b-core/src/nas.rs`（独立仓库） |
| e2b bind 验收 | `deploy/stack/lib/verify-e2b-nas-inject.sh` |

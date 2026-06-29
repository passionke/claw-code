# 架构治理 — FC-only 本地栈

Author: kejiqing

**分支：** `arch/governance-fc-only`

**目的：** 将基础设施与应用栈解耦；本地仅保留 gateway + playground；所有 worker/OVS/Observe/NAS 写盘走 e2b 沙箱；数据按 `cluster_id` 强隔离。

---

## 1. 目标拓扑

| 角色 | 地址 | 说明 |
|------|------|------|
| PostgreSQL | `10.8.0.1:5433` | 独立 compose，与应用零耦合 |
| e2bserver | `10.8.0.1:3000` / `:3002` | 与 PG 同宿主机，不同端口 |
| NAS | `10.8.0.11:/` | 唯一 NFS export；e2b 宿主机 mount → `/mnt/nas0` |
| 本地 dev | gateway + playground | 无 bundled PG、无 NAS volume bind |

**IP 治理（勿混用）：**

| 地址 | 角色 | 常见误写 |
|------|------|----------|
| `10.8.0.1` | PG + e2bserver API `:3000` / envd `:3002` | ~~`10.8.0.9`~~（旧节点，已废弃） |
| `10.8.0.11` | NAS NFS export | ~~`10.8.0.8`~~（旧 NAS 节点） |
| `supone.top` | FC sandbox 浏览器 traffic（wildcard DNS → e2b traffic 入口） | 勿把 `CLAW_FC_DOMAIN` 写成 IP |

脚本 / `.env.example` 的 fallback 必须与上表一致；历史文档（`docs/ovs-chat/*` 踩坑记录）内 `10.8.0.9` 保留为**当时取证**，文首有勘误横幅。

```text
all -> cluster_id -> project -> session -> turn
```

同一 PG 内不同 `CLAW_CLUSTER_ID` 的数据互不可见；`reconcile_interrupted_turns` 仅作用于当前 cluster。

---

## 2. NAS 不变量

- NAS 进 sandbox **仅 host 直 bind**：`{hostMountRoot}/{relPath}` → guest `mountDir`
- e2b 宿主机：`mount -t nfs -o vers=4.2,_netdev 10.8.0.11:/ /mnt/nas0`
- **禁止** sandbox 内 `mount.nfs4`（Firecracker 无 `CAP_SYS_ADMIN`，见 `docs/ovs-chat/FC-OVS-E2E-FAILURES.md` F2）
- Gateway **不** bind-mount NAS；项目资源读写经 **claw-nas-api** e2b singleton

详见 [`fc-nas-workspace.md`](fc-nas-workspace.md)。

---

## 3. 稳定 e2b singleton（1 年租期）

| 组件 | 模板 | clawRole | Admin reset |
|------|------|----------|-------------|
| OVS | `claw-ovs` | `ovs-singleton` | `POST .../ovs-singleton/reset` |
| Observe Tap | `claw-observe` | `observe-singleton` | `POST .../observe-tap/reset` |
| NAS API | `claw-nas-api` | `nas-api-singleton` | `POST .../nas-api/reset` |

Gateway shutdown **不杀** persistent singleton；worker lease ticker 与 singleton lease 分流。

---

## 4. Worker 模式（FC-only）

- **唯一路径：** `CLAW_SOLVE_ISOLATION=fc`、`CLAW_INTERACTIVE_BACKEND=fc`
- **strict：** `claw-worker-strict`（guest `claw` uid）
- **relaxed：** `claw-worker-relaxed`（guest root，需 `CLAW_ALLOW_RELAXED_WORKER=1`）

宿主机 `claw-sandbox` / `podman_pool` / `docker_pool` / `claw-pool-daemon` **已从代码与 deploy 脚本移除**。

---

## 5. 部署命令

```bash
# 10.8.0.1 基础设施（PG）
./deploy/stack/gateway.sh infra-pg-up

# 本地 dev（gateway + playground，外连 10.8.0.1 PG）
cp deploy/stack/env.selfhosted-e2b.example .env   # 编辑 CLAW_CLUSTER_ID / keys
./deploy/stack/gateway.sh quick
```

---

## 6. 迁移 checklist

- [ ] `CLAW_GATEWAY_DATABASE_URL` → `@10.8.0.1:5433`
- [ ] `CLAW_FC_API_URL` / `CLAW_E2B_SANDBOX_URL` → `10.8.0.1`
- [ ] NAS server → `10.8.0.11`；e2b 宿主机 `/mnt/nas0` 已挂载
- [ ] `CLAW_CLUSTER_ID` 已设；旧 PG 数据 migrate 回填 cluster_id
- [ ] e2b 模板：`claw-worker-strict`、`claw-worker-relaxed`、`claw-ovs`、`claw-observe`、`claw-nas-api`
- [ ] `verify-e2b-nas-inject.sh` 通过

---

## 7. 废弃清单

| 路径 / 变量 | 状态 |
|-------------|------|
| `sandbox/`、`claw-sandbox` 二进制 | 已删除 |
| `docker_pool.rs`、`claw-pool-daemon`、`pool-daemon-*.sh` | 已删除 |
| `deploy/stack/docs/host-pool-daemon.md` | 已删除 |
| `CLAW_SANDBOX_*`、`CLAW_POOL_HTTP_*`、`podman_pool`、`docker_pool` | 无消费者；`.env` 可标注废弃 |
| `stable-dev-host-up.sh`、`env.stable-dev-host.example` | 已删除 / 废弃 |
| `e2b-selfhosted-nfs-mount.sh` | 已废弃，勿恢复 |
| compose bundled postgres（本地 quick） | 默认跳过，外连 10.8.0.1 |

**文档索引：** [`docs/README.md`](README.md)

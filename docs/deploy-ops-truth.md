# 运维指令与「真值」验收

Author: kejiqing

避免 **pack-deploy / up 显示成功但仍在跑旧代码**。部署后必须用 **`verify`** 或让 **`pack-deploy`** 自带的验收通过。

## 指令对照

| 指令 | 做什么 | 是否保证新代码 |
|------|--------|----------------|
| **`gateway.sh pack-deploy`** | `build` → `down`+`up` → **`claw-stack-verify`** → `check` | **是**（推荐标准发布） |
| **`gateway.sh quick`** | admin build + playground + `up` + `check` | gateway 镜像若未 build 仍可能旧 |
| **`gateway.sh up`** | 起 gateway + playground compose | 仅当镜像已新 |
| **`gateway.sh build`** | 编译 gateway 镜像 + stamp | 只构建，不重启 |
| **`gateway.sh check`** | healthz + 连通性冒烟 | **不**检查 e2b 模板 |
| **`gateway.sh verify`** | PG schema、FC 配置、gateway 健康 | **必须**用于确认 |

## 标准发布（本机）

```bash
./deploy/stack/gateway.sh pack-deploy
./deploy/stack/gateway.sh verify
```

## verify 检查项（e2b-only 摘要）

1. PG：迁移表存在；`CLAW_CLUSTER_ID` 与连接串一致
2. Gateway `/healthz` 与 `/readyz`（clawTap clusterHash 若启用 strict）
3. **e2b**：`CLAW_E2B_API_URL` 可达；必要模板已 build（见 [`deploy-ops-runbook.md`](deploy-ops-runbook.md)）
4. **跳过**：宿主机 `:9944` pool、`claw-worker-*` 容器、`daemon.log`

## 构建戳

`deploy/stack/.claw-build-stamp.env`：最后一次 `build.sh` 的 git rev / 时间。

## Live SSE

`running`/`queued` 的 live stream 经 gateway LiveReportHub 或 e2b 路径；无 `CLAW_POOL_HTTP_BASE` fallback。见 `docs/live-report-contract.md`。

## See also

- [`deploy-ops-runbook.md`](deploy-ops-runbook.md)
- `deploy/stack/README.md`
- `docs/architecture-governance.md`

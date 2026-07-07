# Relaxed Worker 内置 OVS — 验收契约

Author: kejiqing

## 架构摘要

- **strict 项目**：无 OVS 入口；`GET /v1/projects/{id}/ovs/workspace` → **403**
- **relaxed 项目**：OVS 与 worker **同一 e2b sandbox**（`claw-worker-relaxed` 模板），工作区路径 **`/claw_ds`**
- **废除**独立 `ovs-singleton` sandbox 与 `ensure_ovs` 启动路径

## 不变量（INV）

| ID | 断言 |
|----|------|
| INV-1 | strict 项目 `ovs/workspace` HTTP **403** |
| INV-2 | relaxed 响应 `workspaceFolder == "/claw_ds"` |
| INV-3 | `ovsFolderUrl` 中 sandbox 与 `sandboxId`（= `project_e2b_worker.sandbox_id`）一致 |
| INV-4 | e2b API 上无 `metadata.clawRole=ovs-singleton` 存活 sandbox |
| INV-5 | 响应含 `clusterId`、`workerProfile` |
| INV-6 | OVS 流量 host `3000-sbx_*` 与 worker ttyd `7681-sbx_*` 共享同一 `sbx_*` |
| INV-9 | `ovsFolderUrl` 不得含 legacy `/claw_ws/proj_*` |

## 验证层级

| 层级 | 命令 | 说明 |
|------|------|------|
| L0 | `cargo test -p claw-e2b-sandbox-client nas_paths` | 路径契约单测 |
| L0 | `cargo test -p http-gateway-rs gateway_e2b_ovs` | OVS URL 派生单测 |
| L1 | `./deploy/stack/lib/verify-relaxed-worker-ovs.sh` | Gateway + relaxed OVS 契约 |
| L2 | `CLAW_OVS_BACKEND=e2b ./deploy/stack/lib/verify-e2b-ovs-e2e.sh` | 完整 e2b E2E |
| L3 | `CLAW_OVS_E2E_PROJ_ID=2 ./deploy/stack/lib/verify-ovs-claw-e2e.sh` | @claw agent/ws |

## 发版顺序

```bash
./deploy/e2b/build-claw-worker-relaxed-selfhosted.py
./deploy/stack/lib/verify-relaxed-worker-ovs.sh
CLAW_OVS_E2E_PROJ_ID=2 ./deploy/stack/lib/verify-ovs-claw-e2e.sh
```

## 环境变量

| 变量 | 默认 | 用途 |
|------|------|------|
| `CLAW_OVS_E2E_PROJ_ID` | `2` | relaxed 验收项目 |
| `CLAW_STRICT_E2E_PROJ_ID` | `1` | strict 403 验收项目 |
| `CLAW_E2B_TEMPLATE_RELAXED` | `claw-worker-relaxed` | relaxed worker 模板别名 |

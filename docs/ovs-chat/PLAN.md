# OVS + 插件路线图（实施记录）

Author: kejiqing

产品方向：**OVS + `claw-vscode` 为唯一交互入口**；Playground `/coding` 封存。

操作手册见 [INTEGRATION.md](./INTEGRATION.md)。

---

## 已实现（2026-06）

| 能力 | 说明 |
|------|------|
| Project 对齐 | `claw.projId` 仅来自 Gateway 写入的 workspace settings / 显式配置；**禁止**用文件夹名猜 projId |
| 安装脚本 | 不再在根 `.vscode` 写死 `claw.projId: 1`；`materialize_proj_home` 为 `proj_N/home/.vscode/settings.json` 写入 `claw.projId: N`（仅当文件不存在） |
| Session 注册 | `terminal_start` 对 `ovs-*` session 写入 `gateway_sessions`，`client_origin=ovs-chat` |
| 轮次落库 | 每次 agent WS `prompt` → `gateway_turns`（`client_origin=ovs-chat`）；CDP `content.delta` 拼接为 `report_message` |
| OVS 路径对齐 | 仅 `ovs-*`：`ttyd -w /claw_ds`；`GET .../ovs/workspace` 完整物化；solve worker 仍为 session `cwd` |
| E2E | `verify-ovs-claw-e2e.sh` 支持 `CLAW_OVS_E2E_PROJ_ID=2` |

### Session 命名（现期）

- 每 project 一个 REPL：`ovs-{projId}`（例：`ovs-2`）
- Claw 执行目录：`proj_{N}/sessions/ovs-{N}/`

### 轮次语义

- OVS Chat UI：仍由 VS Code 本地保存多轮历史
- Gateway：每次 `@claw` 发送一条 prompt 产生一行 `gateway_turns`（与 UI 不双向同步）
- 同一 project 下多个 Chat 面板 **共用** 一个 `ovs-{projId}` terminal（共享 REPL 上下文）

---

## FC OVS Singleton（设计中）

**1 Gateway : 1 OVS : N Worker** — OVS 走 e2b 单例（`claw-ovs` template），不进 worker template；Mac compose 不跑 OVS。

→ 完整设计：[FC-OVS-SINGLETON-DESIGN.md](./FC-OVS-SINGLETON-DESIGN.md)

---

## 后续：Git 分支 → 独立 REPL（未实现）

目标：一分支一 REPL，避免跨分支污染 claw 上下文。

### 设计草案

| 项 | 方案 |
|----|------|
| Session ID | `ovs-{projId}-{branchSlug}` |
| branchSlug | git 分支名规范化：非 `[a-zA-Z0-9._-]` → `-`，最长 64 |
| 插件 | 读 `vscode.git` / workspace API 当前分支，拼 `sessionId` |
| Gateway | 无需改路由；新 sessionId 即新 `gateway_sessions` + 新 terminal |
| Pool | 同一 project 多分支可能占多个 slot；需评估 min idle 或 OVS 专用 label |

### 风险

- 分支切换后用户需知「新分支 = 新 claw 会话」
- 长期活跃分支数 × project 数可能压满 pool（dev 继续 `gateway.sh pool-reset`）

### 不在此演进内

- OVS Chat UI ↔ PG transcript 双向同步
- passionke OVS 镜像内 Chat 产品改动（除非 1.109.5 git API 不可用）

---

## 验证命令

```bash
# proj 1（默认）
./deploy/stack/lib/verify-ovs-claw-e2e.sh

# proj 2
CLAW_OVS_E2E_PROJ_ID=2 ./deploy/stack/lib/verify-ovs-claw-e2e.sh
```

Gateway 变更后：`cd rust && cargo test -p http-gateway-rs`，本地 `./deploy/stack/gateway.sh quick` 或 `pack-deploy`。

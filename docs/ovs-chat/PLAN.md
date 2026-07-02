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
- 同一 project 下多个 Chat 面板共用 `ovs-{projId}` **交互 worker**；**多轮 AI 上下文**按 `record_session_id` 写入 `proj_N/sessions/{segment}/interactive-session.jsonl`（B1 resume）；turn 后写回 PG `cc_messages`（**不**从 PG 灌入 prompt）

---

## 已实现：交互式多轮上下文（P0）

**方案：** B1（exec + resume jsonl）；resolve/solve 保持 PG 续聊不变。  
**文档：** [OVS-INTERACTIVE-CONTEXT-PLAN.md](./OVS-INTERACTIVE-CONTEXT-PLAN.md)  
**验收：** `CLAW_OVS_E2E_MULTI_TURN=1 ./deploy/stack/lib/verify-ovs-claw-e2e.sh`（需 live LLM）

---

## e2b OVS Singleton（已实现 P0–P2）

**1 Gateway : 1 OVS : N Worker** — OVS 走 e2b 单例（`claw-ovs` template），不进 worker template；Mac compose 不跑 OVS。

→ 完整设计：[FC-OVS-SINGLETON-DESIGN.md](./FC-OVS-SINGLETON-DESIGN.md)  
→ **NAS 路径与各组件本地视图：** [e2b-nas-workspace.md](../e2b-nas-workspace.md)

## e2b Session 可观测单例（P1 代码已合入）

**1 Gateway : 1 Observe : 1 OVS : N Worker** — e2b 只读 Live 看 sessionId 执行过程；**不做 LLM 代理**；worker 内嵌 tap 不变。

→ 设计：[FC-TAP-SINGLETON-DESIGN.md](./FC-TAP-SINGLETON-DESIGN.md)  
→ 模板：`python3 deploy/e2b/build-claw-observe-selfhosted.py`

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

# OVS + Claw Chat 调试记录本

Author: kejiqing  
用途：claw-code 内快速回顾；**新开源码工程请读主文档**。

---

## 主文档（交接用）

**[docs/ovs-chat-source-handoff.md](./ovs-chat-source-handoff.md)** — 问题结论、调用链、证据表、证伪清单、新工程结构、成功标准。

---

## 当前结论（2026-06-18）

**排障总结（证据链）：** [docs/ovs-chat/INCIDENT-2026-06-18.md](./ovs-chat/INCIDENT-2026-06-18.md)  
**部署契约：** [docs/ovs-chat/EXTENSION-STABLE-DEPLOY.md](./ovs-chat/EXTENSION-STABLE-DEPLOY.md)

| 项 | 状态 |
|----|------|
| OVS 运行时 | `passionke/openvscode-server:1.109.5-ovs-chat`（Podman :13000） |
| 插件验证 | `extensions/claw-vscode` + `ovs-claw-restart.sh` |
| 集成文档 | **[docs/ovs-chat/INTEGRATION.md](./ovs-chat/INTEGRATION.md)** + **[PLAN.md](./ovs-chat/PLAN.md)** |

**demo 关键：** `registerLanguageModelChatProvider` + `createChatParticipant` + `isDefault` + `extensionKind: workspace` only。

主文档仍见 **[docs/ovs-chat-source-handoff.md](./ovs-chat-source-handoff.md)**（历史证据与调用链）。

---

## 目标

- **唯一前进路径：** OVS + `claw-vscode` → gateway agent WS → pool worker claw
- Playground `/coding`：**封存**（本线不再讨论、不扩展）

---

## 时间线（简）

| 阶段 | 结果 |
|------|------|
| HOME/扩展路径、`/opt/claw-extensions` | EH 能加载扩展 |
| 删假 `defaultChatAgent` product patch | 不再 `reading 'response'` |
| `ovs-chat-demo` 隔离 Claw | 证明非 gateway 问题 |
| `isDefault` + proposed API | 修复 `No default agent contributed` |
| Browser / serverless / ESM 尝试 | Install in Browser 失败或 handler 仍不调 |
| 读 1.105.1 源码 + E2E | 断点在 `$invokeAgent` 之后、handler 之前或 RPC 路由 |

---

## 相关文件

- `docs/ovs-chat-source-handoff.md` — **交接主文档**
- `extensions/ovs-chat-demo/`
- `deploy/stack/Containerfile.openvscode`
- `deploy/stack/openvscode-settings.json`
- `deploy/stack/lib/verify-ovs-chat-demo.sh`
- `docs/boundaries-claw-stack.md`

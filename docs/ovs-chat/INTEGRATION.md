# OVS × claw-code 集成手册

Author: kejiqing  
适用：**Podman compose** + `passionke/openvscode-server:1.109.5-ovs-chat` + claw-code Gateway 联调

**产品方向（2026-06）：** 交互入口以 **OVS + `claw-vscode` 插件** 为唯一前进路径；Playground `/coding` **封存**，本手册不再展开 `/coding` 流程。

> **`@claw` 反复不生效时，只读 [EXTENSION-STABLE-DEPLOY.md](./EXTENSION-STABLE-DEPLOY.md)**（安装/cache/projId/Playground 唯一契约）。本文是集成总览。

---

## 0. 架构（单一默认路径）

| 组件 | 镜像 / 路径 | 端口 |
|------|-------------|------|
| OVS | `CLAW_OVS_UPSTREAM_IMAGE`（见 `deploy/stack/ovs-image.env`） | `13000` → `/ovs/` |
| Gateway | `claw-gateway-rs:local` | `.env` `GATEWAY_HOST_PORT`（如 8088） |
| Playground | `claw-gateway-playground:local` | `18765` |
| 工作区 | `deploy/stack/claw-workspace` → `/home/workspace` | bind mount |
| 插件源码 | `extensions/claw-vscode/` | 打包 VSIX 热装进容器 |

OVS 运行时（Chat 派发链）在 **passionke 镜像**；业务扩展在 **claw-code** 迭代。

---

## 1. 日常命令

```bash
# 1. 后端（仓库根）
./deploy/stack/gateway.sh quick

# 2. 装/更新 claw-vscode 并重启 OVS 容器
./deploy/stack/lib/ovs-claw-restart.sh

# 3. 验证
./deploy/stack/lib/verify-claw-vscode.sh   # 必须通过（含 extensions.user.cache）
./deploy/stack/lib/verify-ovs-claw-e2e.sh
# proj 2：CLAW_OVS_E2E_PROJ_ID=2 ./deploy/stack/lib/verify-ovs-claw-e2e.sh
```

`ovs-claw-restart.sh` 重启后会自动跑 proj 1 E2E。  
**发布前：** `verify-claw-vscode.sh` 失败则不要开浏览器，见 [EXTENSION-STABLE-DEPLOY.md](./EXTENSION-STABLE-DEPLOY.md)。

浏览器（关旧标签后）：

- **推荐：** `http://127.0.0.1:18765/ovs?projId=1` → 登录 Playground → 自动跳到 `:13000/?folder=/home/workspace/proj_1/home`
- 直连 OVS：先 `curl -s http://127.0.0.1:8088/v1/projects/1/ovs/workspace`，再开返回的 `workspaceFolder`：
  `http://127.0.0.1:13000/ovs/?folder=/home/workspace/proj_1/home`

Chat → `@claw ping`；Output → **Claw**。

---

## 1.1 Project / session 契约（OVS 侧）

三条须一致，否则会出现「编辑器是 proj_2、Claw 跑在 proj_1」：

| 层 | project 2 示例 | 说明 |
|----|----------------|------|
| OVS 工作区 | `?folder=/home/workspace/proj_2/home` | 编辑器看到的代码树 |
| 插件 `projId` | `2` | **`proj_2/home/.vscode/settings.json`** 里 `claw.projId`（Gateway 写入）；不用文件夹名推断 |
| Gateway WS | `sessionId=ovs-2`，`projId=2` | `ws://gateway-rs:8080/v1/sessions/ovs-2/agent/ws?projId=2` |

**Claw 执行目录**（非 `proj_N/home`）：`proj_2/sessions/ovs-2/`（宿主机在 `deploy/stack/claw-workspace/` 下）。**OVS 交互 REPL** 的 `cwd` 对齐 OVS 工作区（`proj_N/home` → worker `/claw_ds`）；会话 jsonl 仍写在 `HOME=/claw_host_root`（`sessions/ovs-N`）。**solve 等其它 worker 不变**（`cwd` 仍在 session 工作区）。

**契约 API：** `GET /v1/projects/{id}/ovs/workspace` → 物化 `CLAUDE.md` 等 + `workspaceFolder` + `agentSessionId`（`ovs-{projId}`）。

**Session 注册：** 首次 `terminal_start` 写入 `gateway_sessions`，`client_origin=ovs-chat`（`ovs-*` session id）。

**轮次记录：**

- OVS Chat UI：各 Chat 面板本地历史（VS Code 产品内，不与 PG 双向同步）。
- Gateway：每次 agent WS `prompt` → 一行 `gateway_turns`（`client_origin=ovs-chat`）；`report_message` 为 CDP `content.delta` 拼接。
- 同一 project 下多个 Chat 面板 **共用** 一个 `ovs-{projId}` REPL。

路线图与后续（git 分支 → 独立 REPL）：[PLAN.md](./PLAN.md)

自检：Output → **Claw** 应打印 `agent ws url=...projId=2...ovs-2...`。

---

## 2. 镜像与配置

**上游 OVS 镜像**（`deploy/stack/ovs-image.env`）：

```
crpi-cf9vxpq3n8or17mw.cn-hangzhou.personal.cr.aliyuncs.com/passionke/openvscode-server:1.109.5-ovs-chat
```

可在根 `.env` 覆盖 `CLAW_OVS_IMAGE` / `CLAW_OVS_UPSTREAM_IMAGE`。

**可选：烘焙 claw-vscode 进本地 tag**（非日常）：

```bash
CLAW_OVS_IMAGE=claw-openvscode-server:local CLAW_FORCE_REBUILD_OVS=1 ./deploy/stack/gateway.sh build
```

`Containerfile.openvscode` = 上游镜像 + VSIX + Machine settings。

**Machine 设置**：`deploy/stack/openvscode-settings.json`（compose bind mount）

- `claw.gatewayHost`: `gateway-rs:8080`（容器内 Remote EH 连 compose 网关）
- `chat.agent.enabled`: **`true`**（`false` → `No activated agent`）
- **不要**在 Machine settings 写 `claw.projId`（Gateway 写入 `proj_N/home/.vscode`）
- 完整键表：[EXTENSION-STABLE-DEPLOY.md §4](./EXTENSION-STABLE-DEPLOY.md#4-ovs-machine-settings固定文件)

---

## 3. 插件开发循环

改 `extensions/claw-vscode/*` 后：

```bash
./deploy/stack/lib/ovs-claw-restart.sh
```

仅装扩展不重启：`./deploy/stack/lib/install-claw-vscode-container.sh`

---

## 4. 相关文件

| 路径 | 用途 |
|------|------|
| **`docs/ovs-chat/EXTENSION-STABLE-DEPLOY.md`** | **@claw 稳定部署唯一手册（install/cache/projId）** |
| `deploy/stack/ovs-image.env` | 上游 OVS 镜像 pin |
| `deploy/stack/podman-compose.yml` | `openvscode-server` 服务 |
| `deploy/stack/openvscode-settings.json` | Machine Chat / claw 配置 |
| `deploy/stack/lib/install-claw-vscode-container.sh` | VSIX → 容器 |
| `deploy/stack/lib/ovs-claw-restart.sh` | 装扩展 + restart |
| `deploy/stack/lib/verify-ovs-claw-e2e.sh` | agent WS E2E |
| `docs/ovs-chat/PLAN.md` | 路线图 / git 分支演进 |
| `deploy/stack/lib/verify-claw-vscode.sh` | 扩展冒烟 |
| `extensions/claw-vscode/` | 业务扩展源码 |

---

## 5. 历史

- `docs/ovs-chat-source-handoff.md` — 1.105.1 证据链（已过时设置见本文 §2）
- 旧路径：macOS 自编译 `:3100` + `install-claw-vscode-ovs.sh`（保留脚本，非默认）

# claw-vscode 稳定部署契约（OVS @claw）

Author: kejiqing  
**本文是 `@claw` 能否生效的唯一操作手册。** 改插件、改 OVS、排障都先对照本文；不要临时改 compose / Machine settings 猜 projId。

日常集成见 [INTEGRATION.md](./INTEGRATION.md)；路线图见 [PLAN.md](./PLAN.md)。

---

## 1. 一句话结论

| 现象 | 根因（已证实） |
|------|----------------|
| `No activated agent with id "claw.claw"` | ① `chat.agent.enabled: false` ② 只 unzip 未 `--install-extension` ③ **JSDoc `proj_*/home` 含 `*/`** → `extension.js` 语法错误，扩展未 activate |
| `Language model unavailable` | 删掉 **stub LM**（`registerLanguageModelChatProvider` + `chatProvider` proposal）；与上条独立，**两项都要** |
| `claw.projId not set` | 工作区不是 `proj_N/home`；或 Gateway 未 `GET .../ovs/workspace` |
| Explorer 显示整盘 `workspace`（`proj_*`/`ds_*` 并列） | 直接开 `:13000/ovs/` 且曾用 `--default-folder=/home/workspace`；**必须** `?folder=.../proj_N/home` |
| `agent WebSocket error (no gateway response)` | ① OVS **不在 `claw_default`**，`gateway-rs` ENOTFOUND ② 浏览器误连 `:13000/ovs/agent/ws` ③ `ovs-N` ttyd 僵死 |
| `@claw` 无反应 / Working 不结束 | 同上，或浏览器未硬刷新（旧 EH / 旧扩展） |

**稳定原则：** `--install-extension`；projId 只认 Gateway 的 `proj_N/home/.vscode/settings.json`；`chat.agent.enabled: true`；**保留 stub LM**；OVS 与 gateway 同 compose 网络（`claw_default`）。  
**当天完整证据链：** [INCIDENT-2026-06-18.md](./INCIDENT-2026-06-18.md)

---

## 2. 黄金路径（唯一支持）

```bash
# 仓库根
./deploy/stack/gateway.sh quick          # 或 up（栈已在跑可跳过）
./deploy/stack/lib/ovs-claw-restart.sh   # 装扩展 + 重启 OVS + E2E
./deploy/stack/lib/verify-claw-vscode.sh # 必须通过（语法 + folder 302 + chat.agent.enabled）
```

浏览器（**先关旧 OVS 标签，硬刷新**）：

```
http://127.0.0.1:18765/ovs?projId=1
```

登录 Playground 后自动跳到：

```
http://127.0.0.1:13000/ovs/?folder=/home/workspace/proj_1/home
```

Chat → `@claw ping`；View → Output → **Claw** 应出现 `activate()`、`createChatParticipant ok`、`handler prompt=...`。

---

## 3. 扩展安装契约（禁止偏离）

### 3.1 必须做的

脚本：`deploy/stack/lib/install-claw-vscode-container.sh`

1. 打包 VSIX：`extensions/claw-vscode` → `deploy/stack/claw.claw-vscode-<version>.vsix`
2. `podman cp` VSIX 进 `claw-openvscode-server`
3. 在容器内执行：

```bash
/home/.openvscode-server/bin/openvscode-server \
  --install-extension /tmp/claw-vscode.vsix \
  --extensions-dir=/opt/claw-extensions \
  --server-data-dir=/opt/claw-ovs/server-data \
  --force
```

4. 验证（当前 OVS 1.109.5）：

```bash
# A. CLI 列表
podman exec claw-openvscode-server /home/.openvscode-server/bin/openvscode-server \
  --list-extensions --extensions-dir=/opt/claw-extensions \
  --server-data-dir=/opt/claw-ovs/server-data | grep '^claw\.claw-vscode$'

# B. 语法（JSDoc 勿写 proj_* /home，*/ 会炸 activate）
podman exec claw-openvscode-server /home/.openvscode-server/node --check \
  /opt/claw-extensions/claw.claw-vscode-<version>/extension.js

# C. gateway-rs 可达（OVS 须在 claw_default 网络）
podman exec claw-openvscode-server /home/.openvscode-server/node -e \
  "require('dns').lookup('gateway-rs',(e)=>process.exit(e?1:0))"
```

5. 若 C 失败：`podman network connect claw_default claw-openvscode-server`（install 脚本已自动尝试；**勿**裸 `podman-compose up` 不带 `COMPOSE_PROJECT_NAME=claw`）

6. `podman restart claw-openvscode-server`

`ovs-claw-restart.sh` = 上述 install + restart + `verify-ovs-claw-e2e.sh`。

### 3.2 禁止做的

| 做法 | 后果 |
|------|------|
| 只 `unzip` VSIX 到 `deploy/stack/claw-ovs-extensions/` | 磁盘有 `extension.js`，**Chat participant 不注册** |
| 手改 `extensions.json` 冒充已安装 | 同上 |
| 只 `podman restart` 不跑 install | 旧 cache / 旧版本 |
| 在 compose / Machine settings / 环境变量写 `claw.projId` | 多项目错乱；违反 Gateway 契约 |
| 在 `claw-workspace/.vscode` 写 `claw.projId` | 根目录不是项目 home；易与 proj_N 不一致 |

---

## 4. OVS Machine settings（固定文件）

文件：`deploy/stack/openvscode-settings.json`  
挂载：`podman-compose.yml` → `/opt/claw-ovs/server-data/Machine/settings.json`

**Chat 相关必须为：**

```json
{
  "chat.disableAIFeatures": false,
  "chat.agent.enabled": true,
  "chat.experimental.serverlessWebEnabled": false,
  "chat.experimental.disableCoreAgents": true,
  "claw.gatewayHost": "gateway-rs:8080",
  "claw.gatewayPublicHost": "127.0.0.1:8088",
  "claw.playgroundPort": "18765"
}
```

| 键 | 值 | 说明 |
|----|-----|------|
| `chat.agent.enabled` | **`true`** | `false` → `No activated agent with id "claw.claw"` |
| `chat.disableAIFeatures` | `false` | `true` 会关掉 Chat 能力 |
| `claw.gatewayHost` | `gateway-rs:8080` | **Remote EH** 直连 compose 网关（须同 `claw_default` 网络） |
| `claw.gatewayPublicHost` | `127.0.0.1:8088` | **浏览器** 开在 `:13000` 时的 agent WS（勿连 `:13000/ovs/agent/ws`） |
| `claw.playgroundPort` | `18765` | 浏览器开在 Playground 时用 `/ovs/agent/ws` 代理 |
| `claw.projId` | **不要写** | projId 见 §5 |

compose 启动参数（勿删；**不要**加 `--default-folder=/home/workspace`）：

```yaml
--extensions-dir=/opt/claw-extensions
--server-data-dir=/opt/claw-ovs/server-data
--enable-proposed-api=claw.claw-vscode
```

改 `openvscode-settings.json` 后：`podman restart claw-openvscode-server`。

---

## 5. projId 契约（Gateway 唯一来源）

| 谁写 | 写哪里 | 何时 |
|------|--------|------|
| Gateway | `proj_N/home/.vscode/settings.json` → `claw.projId: N` | `GET /v1/projects/N/ovs/workspace`、`materialize`、栈 bootstrap |
| 插件读 | 当前 workspace folder 的 settings；或 Playground 打开的 `proj_N/home` | `@claw` 时 |

```bash
curl -s "http://127.0.0.1:8088/v1/projects/1/ovs/workspace" | jq .
# workspaceFolder: "/home/workspace/proj_1/home"
```

**工作区必须对齐：** 资源管理器根目录是 `proj_1/home` 里的文件，不是 `ds_*` / `proj_*` 并列那一层。

---

## 6. Playground 入口契约

文件：`web/gateway-async-playground/server.py`

- 用户访问 `http://127.0.0.1:18765/ovs?projId=N`（需登录）
- Playground 调用 Gateway `GET /v1/projects/N/ovs/workspace`（materialize + `claw.projId`）
- **302 重定向**到 `PLAYGROUND_PUBLIC_OVS_BASE`（默认 `http://127.0.0.1:13000`）+ `?folder=/home/workspace/proj_N/home`

**不要**指望 `:18765` 代理 HTML 能让 OVS 打开正确子目录。

改 `server.py` 后需更新 playground 容器：

```bash
./deploy/stack/gateway.sh build local   # 重建 claw-gateway-playground:local
./deploy/stack/gateway.sh up            # 或 podman restart claw-gateway-playground（仅热贴 server.py 时临时用）
```

---

## 7. 自动化验证（发布前必过）

```bash
./deploy/stack/lib/verify-claw-vscode.sh    # 扩展 + cache + chat.agent.enabled + HTTP
./deploy/stack/lib/verify-ovs-claw-e2e.sh   # 容器内 agent WS ping（proj 1）
CLAW_OVS_E2E_PROJ_ID=2 ./deploy/stack/lib/verify-ovs-claw-e2e.sh
```

`verify-claw-vscode.sh` 失败时 **不要** 打开浏览器手工试；先修 install/cache/settings。

---

## 8. 排障决策树

```
@claw 报错 / 无反应
├─ verify-claw-vscode.sh 失败？
│  ├─ extension.js syntax → 修 JSDoc（勿 proj_* /home）+ ovs-claw-restart.sh
│  ├─ chat.agent.enabled != true → 修 openvscode-settings.json + restart OVS
│  └─ Playground 302 无 folder → 修 server.py / 重建 playground
├─ claw.projId not set？
│  ├─ curl GET .../ovs/workspace
│  └─ URL 必须 folder=.../proj_N/home（用 §2 Playground 入口）
├─ Language model unavailable？
│  └─ 扩展缺 stub LM + chatProvider → 恢复 §6 扩展要点，勿只修 participant
├─ No activated agent / activate failed？
│  ├─ node --check extension.js
│  └─ 关标签硬刷新；Output → Claw 应有 activate()
├─ agent WebSocket error？
│  ├─ podman exec OVS dns.lookup('gateway-rs') → ENOTFOUND → network connect claw_default
│  ├─ 浏览器在 :13000 → 应用走 gatewayPublicHost :8088（扩展 ≥0.2.6）
│  └─ E2E 也失败 → terminal/stop+start 或 gateway.sh pool-reset
└─ E2E OK 但浏览器仍失败？
   └─ 硬刷新 / 无痕；见 INCIDENT-2026-06-18.md §2
```

---

## 9. 与 Rust / pool 变更的边界

| 变更类型 | 需要 |
|----------|------|
| 只改 `extensions/claw-vscode/*` | `ovs-claw-restart.sh` |
| 改 `openvscode-settings.json` | `podman restart claw-openvscode-server` |
| 改 `server.py`（Playground 重定向） | 重建 playground 镜像或热贴 + restart |
| 改 gateway Rust（agent / ovs API） | `gateway.sh build local` + `up`；OVS 交互改 ttyd/挂载时 **`pool-reset`** |

---

## 10. 历史教训（勿重复）

详见 **[INCIDENT-2026-06-18.md](./INCIDENT-2026-06-18.md)**。摘要：

1. 只 unzip → participant 不注册；必须 `--install-extension` + `node --check`  
2. `chat.agent.enabled: false` → `No activated agent`  
3. JSDoc `proj_*/home` → 语法错误 → 同上  
4. 删 stub LM → `Language model unavailable`（与 3 无关，都要保留）  
5. compose/Machine 写死 `projId` → 多项目乱  
6. Playground 代理 HTML 无 folder → 302 到 `:13000/?folder=...`  
7. `--default-folder=/home/workspace` → 直开 `:13000/ovs/` 整盘 workspace（已删）  
8. OVS 落错网络（`stack_default`）→ `gateway-rs` ENOTFOUND → agent WS 全挂  
9. 浏览器在 `:13000` 误连 `/ovs/agent/ws` → 改 `gatewayPublicHost`（0.2.6+）  
10. E2E 过 ≠ 浏览器好；必须浏览器 `@claw ping`

---

## 11. 相关文件索引

| 路径 | 角色 |
|------|------|
| `deploy/stack/lib/install-claw-vscode-container.sh` | **唯一**扩展安装实现 |
| `deploy/stack/lib/ovs-claw-restart.sh` | 日常一键 |
| `deploy/stack/lib/verify-claw-vscode.sh` | 扩展冒烟 + folder 302 |
| `deploy/stack/openvscode-settings.json` | Machine Chat 开关 |
| `deploy/stack/podman-compose.yml` | OVS / Playground 服务 |
| `web/gateway-async-playground/server.py` | Playground → OVS 重定向 |
| `rust/.../session_ovs_api.rs` | `claw.projId` 写入 Gateway |
| `docs/ovs-chat/INCIDENT-2026-06-18.md` | **当天排障证据链（必读）** |
| `extensions/claw-vscode/` | 插件源码（≥0.2.6：LM stub + agentWsUrl） |

# FC `claw-ovs` 模板构建说明

Author: kejiqing  
Related: [FC-OVS-SINGLETON-DESIGN.md](./FC-OVS-SINGLETON-DESIGN.md), `deploy/fc-sandbox/build-claw-ovs-selfhosted.py`

---

## 1. 目标

在自建 e2b（`CLAW_OVS_BACKEND=fc`）上提供 **1 Gateway : 1 OVS** 单例沙箱：

- 沙箱内跑 `openvscode-server`（`--server-base-path=/ovs`）
- NAS export 根 bind 到 `/claw_ws`
- Gateway `GET /v1/projects/{id}/ovs/workspace` 返回可打开的 `ovsFolderUrl`

验收（标准路径）：

```bash
./deploy/stack/lib/verify-fc-ovs-e2e.sh
# 或
curl -fsS http://127.0.0.1:${GATEWAY_HOST_PORT:-8088}/v1/projects/1/ovs/workspace
```

---

## 2. 职责边界（claw-code vs e2b）

| 层 | 谁负责 | 做什么 |
|----|--------|--------|
| **claw-code** | 模板**内容** | 从 OVS 上游镜像 staging `openvscode-server` / extensions / VSIX，打成 bundle，定义 debian 层与 NFS sudo 契约 |
| **e2bserver** | 模板**标准化打包** | `Template.build` → 上传 build context（R2）→ GitHub Actions / 节点拉镜像 → 注册别名 `claw-ovs`、调度沙箱 |
| **Gateway** | 运行时编排 | `FcOvsSingleton` 创建沙箱、exec 启动脚本、探活 `http://127.0.0.1:3000/ovs/` |

**e2b 不负责**修正 `openvscode-server` 树里的 native 模块架构；**claw-code 不负责** e2b 节点调度与镜像 registry。

---

## 3. 为何必须本机参与（已确认）

OVS 模板体积大（bundle 约 **70+ MiB**，含完整 `openvscode-server` 树），且对 **linux/amd64 native `.node`** 敏感。

| 约束 | 说明 |
|------|------|
| **GitHub Actions 全远程打 OVS 包不可行** | 实测 e2b 派发的 template CI **1 小时仍打不完**；且无法在 CI 内复用本机已验证的 podman staging 流程 |
| **本机 staging 是标准入口** | 在 **Mac / 开发机** 用 `podman pull --platform linux/amd64` + `podman cp` 从上游镜像抽出目录，再 `Template.build` 上传 bundle（SDK `copy`，非 LAN HTTP） |
| **禁止绕过硬编码模板 ID** | 运行时只用别名 `CLAW_FC_OVS_TEMPLATE=claw-ovs`；临时改 `tpl_*` 不算修复 |

构建命令（repo 根，`.env` 已设 `CLAW_FC_*`）：

```bash
set -a && source .env && set +a
# 可选：.venv-fc 含 e2b SDK（fc-tap-live-up 同款）
python3 deploy/fc-sandbox/build-claw-ovs-selfhosted.py
```

脚本流程：

1. `podman pull/create --platform linux/amd64` ← `CLAW_OVS_IMAGE` / `CLAW_OVS_UPSTREAM_IMAGE`
2. `podman cp` → `openvscode-server`、`claw-extensions`、`claw-ovs` + `claw-vscode` VSIX
3. 本机扫描 `openvscode-server/**/*.node`：必须是 Linux ELF x86-64（明确的 Windows 平台 payload 除外）
4. 打 `claw-ovs-bundle.tar.gz`
5. `e2b.Template.build(alias='claw-ovs', copy bundle, debian unpack)` → 自建 e2b API

当前脚本会 **fail fast**：如果 staged OVS 树里仍有 Mach-O / 非 x86-64 `.node`，直接在本机报错并列出文件，**不会**上传到 e2b build。

**不要**再走已废弃的 `curl http://10.8.0.2:18889/...` 本机临时 HTTP：远端 GitHub builder **连不到** 开发机 LAN，会导致 `curl (28)` / `tar: Error is not recoverable`。

---

## 4. 当前故障与根因（2026-06-26 取证）

### 4.1 现象

| 检查 | 结果 |
|------|------|
| `POST /sandboxes` `templateID=claw-ovs` | **可创建**（e2b 调度 / 别名已恢复） |
| `GET /v1/projects/1/ovs/workspace` | **500**：`fc ovs: openvscode /ovs/ timeout` |
| NAS `/claw_ws/.claw-ovs.log` | 有 `Web UI available at http://localhost:3000/ovs`，同时 **`spdlog.node: invalid ELF header`** |

### 4.2 根因（模板内容，非 e2b 调度）

从上游镜像 `passionke/openvscode-server:1.109.5-ovs-chat-amd64` 抽出文件检查：

| 文件 | 期望 | 实测 |
|------|------|------|
| 镜像 metadata | `amd64 linux` | `amd64 linux` ✓ |
| `node` | ELF x86-64 | ELF x86-64 ✓ |
| `node_modules/@vscode/spdlog/build/Release/spdlog.node` | ELF x86-64 | **Mach-O arm64** ✗（magic `cffa edfe`） |

结论：上游 OVS 镜像在构建时把 **Mac arm64** 的 native 依赖混进了号称 amd64 的镜像；`claw-ovs` 模板原样打包后，Linux 沙箱内 node 加载失败 → Gateway 30s 内 `curl -fsS http://127.0.0.1:3000/ovs/` 探活失败。

### 4.3 待办（claw-code 侧）

在 **本机 staging** 阶段对 `openvscode-server` 树做 **linux/amd64 native 模块校正**（例如：在 amd64 容器内 `npm rebuild` / 重装 `@vscode/spdlog` 等），再 `Template.build`。**不要**指望 e2b CI 单独重打 debian 层能修复 bundle 内已损坏的 `.node`。

---

## 5. 上游镜像与环境变量

| 变量 | 默认 / 说明 |
|------|-------------|
| `CLAW_OVS_UPSTREAM_IMAGE` | `…/openvscode-server:1.109.5-ovs-chat-amd64` |
| `CLAW_OVS_IMAGE` | 覆盖上游镜像 |
| `CLAW_FC_OVS_TEMPLATE` | **`claw-ovs`**（e2b 别名） |
| `CLAW_FC_API_URL` / `CLAW_FC_API_KEY` / `CLAW_FC_DOMAIN` | 自建 e2b API |
| `CLAW_CONTAINER_RUNTIME` | `podman`（staging 用） |

FC/E2B OVS 模板走本文 + `build-claw-ovs-selfhosted.py`；项目运行依赖 FC/E2B 组件。

---

## 6. 运行时备注（Gateway 探活）

启动脚本：`rust/crates/http-gateway-rs/src/pool/interactive_backend/fc_interactive_materialize.rs` → `start_ovs_server_sh`。

- 探活：`curl -fsS http://127.0.0.1:3000/ovs/`（30×1s）
- 日志 / pid：**NAS 共享** `/claw_ws/.claw-ovs.log`、`.claw-ovs.pid`（多沙箱 recreate 时注意陈旧 pid；属运行时细节，与模板 ELF 问题独立）

---

## 7. 验证清单

```bash
# 1) e2b 健康（模板 present）
curl -sS http://supone.top:3000/health | python3 -m json.tool | rg claw-ovs

# 2) 能建沙箱
curl -sS -H "X-API-Key: $CLAW_FC_API_KEY" -H 'Content-Type: application/json' \
  -X POST http://supone.top:3000/sandboxes \
  -d '{"templateID":"claw-ovs","timeout":120}'

# 3) Gateway 工作区 API
curl -fsS http://127.0.0.1:8088/v1/projects/1/ovs/workspace | python3 -m json.tool

# 4) 标准 E2E
./deploy/stack/lib/verify-fc-ovs-e2e.sh
```

通过标准：`ovs/workspace` 返回 `ovsUrl` + `ovsFolderUrl`；浏览器可开 folder；`verify-fc-ovs-e2e.sh` 无 timeout。

---

## 8. 文档索引

| 文档 | 内容 |
|------|------|
| [FC-OVS-SINGLETON-DESIGN.md](./FC-OVS-SINGLETON-DESIGN.md) | 1:1 OVS 单例架构、NAS、API |
| [fc-nas-workspace.md](../fc-nas-workspace.md) | NAS 路径契约 |
| `deploy/fc-sandbox/README.md` | FC 交互总览 + 本模板构建入口 |
| `deploy/fc-sandbox/build-claw-ovs-selfhosted.py` | 构建脚本实现 |

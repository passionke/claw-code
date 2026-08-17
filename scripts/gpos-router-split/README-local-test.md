# Router / Delegate 本机验收（不碰预发）

Author: kejiqing

**约束：**

- **禁止**修改预发 252、`prod-claw-252`、271 等现网/预发 project 配置
- 本机起 gateway（`127.0.0.1:18088`），**借用 94 侧 e2b worker fleet**（`env.selfhosted-e2b.example` 同款 API）
- 数据隔离：`CLAW_CLUSTER_ID=router-test-local`（见 `deploy/stack/env.local-router-test.example`）
- 测试 project 在本机 gateway **新建**（默认 `99010` router / `99011` kb / `99012` ops）

## 1. 本机 gateway

```bash
cp deploy/stack/env.local-router-test.example .env
# 从 deploy/stack/env.selfhosted-e2b.example 合并 CLAW_E2B_*、PG URL、API key

# Mac 原生 binary 调试（不经 compose）推荐额外 env：
export CLAW_GATEWAY_LOG_DIR=/tmp/claw-log
export CLAW_E2B_OBSERVE=0              # observe singleton 超时时可跳过
export CLAW_GATEWAY_SKIP_DB_MIGRATE=1  # 首次 migrate 完成后
export CLAW_HTTP_ADDR=0.0.0.0:18088
export CLAW_WORK_ROOT=/tmp/claw-workspace-local-dev

# live solve 还需 podman machine 运行（session tree chown）
podman machine start

# 或 compose 路径：
./deploy/stack/gateway.sh pack-deploy local
./deploy/stack/gateway.sh up
curl -fsS http://127.0.0.1:18088/healthz | head -c 200
```

**注意：** e2b worker 内 `delegate_project` 回调 `CLAW_GATEWAY_BASE`（`.env` 常为 `http://10.22.28.145:18088`），须确保 sandbox 能访问该地址；仅 `127.0.0.1` 监听时 worker 无法回调。

## 2. 创建测试 project + delegate-targets

```bash
export GW=http://127.0.0.1:18088
export CLAW_ADMIN_TOKEN=...   # 本机 Admin MCP token

./scripts/gpos-router-split/apply_local.sh
```

脚本会：

1. `POST /v1/projects` 创建 router / kb-qa / ops-analysis（若不存在）
2. `PUT .../role` → router seed
3. `PUT .../delegate-targets`
4. activate router（物化 registry 附录）

## 2b. kb-qa 项目配置 + 手册 KB（99011）

`apply_local.sh` **不会**给 99011 配 skill/KB。验收手册类问题前须执行：

```bash
export GW=http://127.0.0.1:18088   # 或 10.22.32.113:18088
export CLAW_ADMIN_TOKEN=...

./scripts/gpos-router-split/apply_kb_local.sh
```

脚本会：

1. 从预发 NAS 快照拉双语手册（`en` 141 + `th` 140 页，只读 rsync，不改 271 配置）
2. 合并 Git 内部文档 + **Mind FAQ**（[产品部 / Faq 文件夹](https://mind.maxiot-inc.com/folders/71bdd401-5be8-4005-996d-53b0f4287c40)）到 `en/internal/mind/faq/`
3. Admin draft → commit → activate（`product-manual-qa` + kb 专用 CLAUDE + 限制 tools）
4. rsync KB 到 94 e2b NAS `proj_99011/home/kb`
5. reset 99011 worker

## 3. 冒烟 / 验收场景

```bash
export GPOS_PROJ_ID=99010   # router
python3 scripts/gpos-router-split/acceptance_smoke.py --scenario 1
python3 scripts/gpos-router-split/acceptance_smoke.py --scenario 4
```

场景定义见 [`docs/specialist-router-acceptance.md`](../../docs/specialist-router-acceptance.md)。

## 4. 与预发脚本的区别

| 脚本 | 目标 | 何时用 |
|------|------|--------|
| `apply_local.sh` | `127.0.0.1:18088` + 本机 cluster | **日常开发验收** |
| `apply_pre.sh` | 预发 GW | **仅运维发布窗口**；本任务 **不要跑** |

## 5. 证据链

验收通过需保留：

- `acceptance-smoke-*.jsonl` 输出
- SQL：`gateway_delegate_session_link` 行（root / parent / delegate sid）
- router turn `succeeded` + 单 SSE 连接

## 6. 自验记录（2026-08-14，local-dev @ 127.0.0.1:18088）

| 项 | 结果 | 证据 |
|----|------|------|
| 022 迁移 | 通过 | log：`CREATE TABLE gateway_delegate_target/session_link` |
| apply_local 建 proj 99010/11/12 | 通过 | PUT role + delegate-targets |
| config 检查 | 通过 | `acceptance_smoke --check-config` pass |
| session sid 复用 | 通过 | 同 parent 两次 resolve，`secondCreated=false` |
| allowlist 负例 | 通过 | projId=999999 → HTTP 400 |
| registry 物化附录 | 通过 | `proj_99010/home/skills/specialist-registry/SKILL.md` 含 target 99011/99012 表 |
| activate API | 脚本已修正 | 正确路径：`POST .../versions/{contentRev}/activate` |
| live solve 场景 1 | **阻塞** | Mac 原生 GW：`podman chown session mount` 失败（需 podman machine + 或 compose 部署） |
| delegate-targets 鉴权 | **待加固** | 无 Bearer 亦可 GET/POST resolve-session |

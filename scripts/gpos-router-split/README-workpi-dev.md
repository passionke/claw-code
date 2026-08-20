# workPi Dev 验收 Runbook

Author: kejiqing

日常 feature 开发以 **workPi**（`passionke@10.22.28.173`）为主力验证环境。与 release 升 tag 流程分离；本 runbook 只覆盖 **branch CI + workPi 一条线**。

## 环境常量

| 项 | 值 |
|----|-----|
| SSH | `passionke@10.22.28.173` |
| Repo | `~/claw_code` |
| Gateway | `http://10.22.28.173:18088` |
| Cluster | `local-dev` |
| e2b API | `http://ailab.spone.xyz:3300`（94 fleet） |
| PG | `10.22.28.94:5433` |
| Router / kb / ops | 99010 / 99011 / 99012 |
| Worker | ACR amd64 `claw-code:<branch-tag>` → e2b 模板 |
| Gateway | workPi **arm64 本地** `gateway.sh build local`（不用 ACR amd64 gateway） |

## A. 发布对齐（一条线）

| 步 | 在哪做 | 命令 / 动作 | 成功标志 |
|----|--------|-------------|----------|
| A1 | GitHub | push 分支 → `claw-code-branch-worker` 跑绿 | ACR 有 `claw-code:branch-<分支名>` |
| A2 | workPi | `git pull` 同分支 | 代码含本次改动 |
| A3 | workPi | `cp -n .env.workpi .env`（首次）后 `./deploy/stack/lib/workpi-branch-deploy.sh branch-<分支名>` | e2b strict+relaxed 模板 PG `buildId` 更新；gateway `build local` + `restart` |
| A4 | workPi | `curl http://127.0.0.1:18088/healthz` | `ok=true`；worker 日志含新 `buildId` |

Tag 示例：`branch-feat-delegate-output-yield`（勿用 `release-v*`）。

## B. 三层验收（harness 合同）

**入口**：Admin `http://10.22.28.173:18088` → proj **99010** router，开 **live** 视图（`biz_advice_report?stream=true`，仍是 router sessionId/turnId，网页不换 URL）。

| 层 | 你怎么看 | 通过标准 | 失败信号 |
|----|----------|----------|----------|
| 2 子占窗口 SSE | Admin 卡片流式区 | delegate 后打字机来自 specialist（手册/问数），同一条 SSE 不断 | 长时间空白后整段跳出；或只有 router 自说自话 |
| 3 主 agent 可读 | `GET .../turns/{turnId}/tools?proj_id=99010` | 每条 `delegate_project` 的 **output 含 `message` 正文** | 只有薄 JSON、无正文 |
| 1 事件环终稿 | 对比：SSE 累加全文 vs `GET /v1/tasks/{sessionId}` → `outputJson.message` vs PG `gateway_turns.report_message` | **三者一致** | 末尾多出「已转交…请稍候」且与上文矛盾；SSE 与 task 分裂 |

### 必跑场景

**场景 4（混合一句）**

- Prompt：`怎么在后台添加商品还有昨天销售额多少？`
- 期望：两次 `delegate_project`（99011 + 99012）；SSE 上 **两段** 打字机；终稿 **无**「稍后提供操作步骤」类话术
- SQL：`gateway_delegate_session_link` 两行，`root_session_id` = router session

**场景 1（单意图续聊）**

- Prompt：`后台怎么创建商品分类？`（连问 2 轮同 session）
- 期望：delegate → kb；SSE 打字机；tool result 含手册正文；续聊 `S_kb` 复用

### 自动化

```bash
export GW=http://10.22.28.173:18088
export CLAW_ADMIN_TOKEN=…
python3 scripts/gpos-router-split/acceptance_smoke.py --scenario 1
python3 scripts/gpos-router-split/acceptance_smoke.py --scenario 4
```

或 `gateway.sh solve-e2e 99010 "…"` 作 API 冒烟，Admin 肉眼验 SSE。

## C. 必留证据

```text
- CI run URL + ACR tag
- e2b-worker-deploy 日志：strict/relaxed templateId + buildId
- 场景 sessionId / turnId
- tools API：delegate 条数 + output 是否含 message
- Admin 终稿截图或复制全文
- 若分裂：SSE delta log 与 task JSON 各存一份
```

## D. 不通过时分层归因

| 现象 | 先查哪层 |
|------|----------|
| 无打字机 / SSE 空 | 2：`delegate.active`、gateway fan-in 日志 `router_fanin_sse` |
| 有打字机但 tool 无正文 | 3：`delegate_project` 是否恢复 `message` |
| 打字机 OK 但终稿多「请稍候」 | 1：prompt 是否复述；是否还有 sidecar 覆盖终稿 |
| ops 段垃圾 | 子 agent / SQLBot（**非 router 验收主项**） |

## 禁止

- Mac/laptop 打 94 fleet e2b 模板（arm64 宿主 + amd64 worker → 用 CI 镜像）
- 只 `gateway restart` 不 `e2b-worker-deploy` 却期望 sandbox claw 变
- 只改 gateway 不推 branch CI 却期望 worker 内 `delegate_project` 行为变
- 用宿主机 checkout 解释线上行为（见 `release-runtime-truth`）

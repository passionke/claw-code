# Sunmi GitLab CLI（glab）与 CI 盯盘

Author: kejiqing

Sunmi 代码与 CI 在 **`http://code.sunmi.com/minidata/claw-code`**；本机用 **`glab`** 查 pipeline / job 日志，**不用**浏览器也能闭环。Agent 修 CI 时必须先拉 trace 再给结论。

---

## 1. 安装与登录（一次性）

```bash
# macOS
brew install glab

# 登录 Sunmi GitLab（浏览器或 Personal Access Token）
glab auth login --hostname code.sunmi.com

# 验收
glab auth status
# 期望：Logged in to code.sunmi.com as <user>
#       REST API Endpoint: http://code.sunmi.com/api/v4/
```

**PAT（Settings → Access Tokens）**：勾选 `read_api`（看 job trace 够用）；写 MR 再加 `api` / `write_repository`。

配置文件：`~/Library/Application Support/glab-cli/config.yml`（macOS）。

---

## 2. Git remote（push 触发 CI 用 `sunmi`）

| Remote | 用途 |
|--------|------|
| **`sunmi`** | `git@code.sunmi.com:minidata/claw-code.git` — **Sunmi CI 只看这条** |
| `origin` | GitHub `passionke/claw-code` — pre-push hook 对比基线可能是 `origin/main` |

```bash
git remote -v
git push sunmi main          # 或 feature 分支；push 即触发 pipeline
```

---

## 3. 常用 glab 命令

项目路径 API 编码：`minidata/claw-code` → `minidata%2Fclaw-code`。

```bash
# 最近 pipeline（按分支）
glab ci list --repo minidata/claw-code --per-page 10

# 某分支最新一条 pipeline id + 状态
glab api "projects/minidata%2Fclaw-code/pipelines?ref=main&per_page=1" \
  | python3 -c 'import json,sys; d=json.load(sys.stdin); print(d[0]["id"], d[0]["status"], d[0]["sha"][:8])'

# 某 pipeline 下所有 job（status / stage / name / id）
PIPELINE_ID=123456
glab api "projects/minidata%2Fclaw-code/pipelines/${PIPELINE_ID}/jobs" \
  | python3 -c "import json,sys; [print(j['status'], j['stage'], j['name'], j['id']) for j in json.load(sys.stdin)]"

# 拉 job 完整日志（JOB_ID 见 UI 或上一条命令）
JOB_ID=214370
glab api "projects/minidata%2Fclaw-code/jobs/${JOB_ID}/trace" | tail -80

# 只看失败行
glab api "projects/minidata%2Fclaw-code/jobs/${JOB_ID}/trace" | rg -i 'error|fail|panic'
```

**浏览器 job 链接**（把 `JOB_ID` 换成数字）：

`http://code.sunmi.com/minidata/claw-code/-/jobs/JOB_ID`

---

## 4. 自动盯 pipeline（推荐）

仓库脚本（依赖已登录的 glab）：

```bash
# 盯当前分支，直到 success/failed；失败时自动 tail 失败 job trace
./deploy/stack/lib/ci-watch-pipeline.sh main

# 环境变量
# CLAW_CI_WATCH_POLL_SEC=30      轮询间隔
# CLAW_CI_WATCH_TIMEOUT_SEC=7200  最长等待（默认 2h）
```

**Agent / 开发闭环**：`git push sunmi <branch>` → `./deploy/stack/lib/ci-watch-pipeline.sh <branch>` → 失败则 `glab api …/jobs/JOB_ID/trace` → 修 → 再 push。

---

## 5. 当前 CI 契约（`.gitlab-ci.yml`）

Push 任意分支 → **同一 `resource_group: claw-deploy`**（串行，避免 runner 上 compose 打架）：

| Stage | Job | 做什么 |
|-------|-----|--------|
| build | `build:release-images` | `render-env-from-ci.sh` → `gateway.sh build --no-clean local` → tag `local/claw-*:${CLAW_RELEASE_TAG}` |
| deploy | `deploy:release` | 见下表 |

**`deploy:release` 脚本顺序**（任一步失败即红；**不得**只验 healthz 宣称通过）：

1. `gateway.sh up --release ${CLAW_RELEASE_TAG}`
2. `gateway.sh verify`
3. `admin-solve-e2e.sh 1 ping` × **2 轮**（node A，:18088 / pool-sunmi-ci-01）
4. **`ci-cluster-dual-deploy.sh`** — node B（独立 `claw-workspace-ci-b`）→ **`cluster-verify`** → **`ci-cluster-solve-e2e`**

Node B 要点：

- **PG**：共用 `claw-gateway-postgres`
- **Workspace**：**独立** `deploy/stack/claw-workspace-ci-b`（session 串台时 gateway 按 PG 路径本地重建目录）
- **Pool**：`pool-sunmi-ci-02`（:9964）

本地只验 node B env 生成（不起服务）：

```bash
# 需已有 repo 根 .env + deploy/stack/.claw-image-release.env（或先 pack-deploy / up --release）
./deploy/stack/lib/ci-cluster-dual-deploy.sh --dry-run
```

---

## 6. Runner 与探测

| 项 | 值 |
|----|-----|
| Runner 宿主机 | `10.22.28.94`（`CLAW_POOL_ADVERTISE_HOST`） |
| Node A 网关 | `http://10.22.28.94:18088/healthz` |
| Node A pool | `http://10.22.28.94:9944/healthz/live-report` |
| Node B 网关（CI） | `http://10.22.28.94:18089/healthz` |
| Admin / Playground | `http://10.22.28.94:18765/` |
| 工作目录 | `/home/gitlab-runner/builds/.../minidata/claw-code`（PG 数据保留：`deploy/stack/claw-postgres-data`） |

同网段快速探测：

```bash
curl -fsS "http://10.22.28.94:18088/healthz" | head -c 200
curl -fsS "http://10.22.28.94:18089/healthz" | head -c 200
curl -fsS "http://10.22.28.94:9944/healthz/live-report" | head -c 200
```

---

## 7. 近期 cluster CI 失败流水（便于对照 commit）

| Job | 现象 | 根因 | 修复 commit |
|-----|------|------|-------------|
| 214348 | `verify` strict registry | `daemon.log` 历史行误判 | `b1376007` |
| 214354 | dual deploy | `GATEWAY_IMAGE` 未从 `.claw-image-release.env` 读 | `1657c41b` |
| 214368 | dual deploy | `POOL_B` 未定义 | `19ce2cbf` |
| 214370 | dual deploy | gateway-only 误 `up postgres` | `cafead3e` |
| 214373 | dual deploy | node B 解析不了 `postgres` | `@claw-gateway-postgres:5432` + pg 网络 |
| 214377 | dual deploy | `host.docker.internal:5433` 连接 hang | `@claw-gateway-postgres:5432` |
| 214380 | deploy | 上轮 node B 残留导致 node A PG hang | `ci-cluster-cleanup.sh` |

新 job 失败：用 **§3** 拉 trace，对照 **§5** 步骤看卡在哪一行，再查对应脚本。

---

## 8. 相关文档

| 文档 | 内容 |
|------|------|
| [`gitlab-ci-variables.md`](gitlab-ci-variables.md) | GitLab CI/CD Variables |
| [`gitlab-ci-troubleshoot.md`](gitlab-ci-troubleshoot.md) | deploy 失败关键字、迁移/ pool 坑 |
| [`cluster-deploy-verify.md`](cluster-deploy-verify.md) | 预发/生产 `cluster-verify` |
| [`docs/architecture-governance.md`](../../../docs/architecture-governance.md) | e2b 拓扑 / 迁移 |
| [`deploy/e2b/README.md`](../../e2b/README.md) | e2b 503 / 模板 |

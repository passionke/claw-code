# GitLab CI 环境变量（code.sunmi.com）

在仓库 **Settings → CI/CD → Variables** 配置；job 跑 `./deploy/stack/lib/render-env-from-ci.sh` 生成仓库根 `.env`，**不要在 runner 上手写 `.env`**。

Author: kejiqing

## 1. 必须在 GitLab UI 配置的变量（Masked）

| Key | Masked | Protected | 说明 | 示例 |
|-----|--------|-----------|------|------|
| `CLAW_BOOTSTRAP_LLM_API_KEY` | **是** | 否（除非 main protected） | LLM API Key；`up` 时写入 PG active LLM | `sk-...` |
| `CLAW_BOOTSTRAP_LLM_BASE_URL` | **否** | 否 | OpenAI 兼容 base URL，**须含 `/v1`**；URL 含 `://` `/` **不能 Mask**，否则 job 可能读不到 | `https://dashscope.aliyuncs.com/compatible-mode/v1` |

`deploy:release` 设了 `CLAW_CI_REQUIRE_LLM_BOOTSTRAP=1`，缺上述两项时 `render-env-from-ci.sh` 直接失败并提示本页。

## 2. 建议在 GitLab UI 配置（可选）

| Key | 说明 | 默认（未设时） |
|-----|------|----------------|
| `CLAW_BOOTSTRAP_LLM_MODEL_NAME` | 模型 id；**不要 Mask** | `gpt-4o-mini` / `qwen-plus` |
| `CLAW_BOOTSTRAP_LLM_NAME` | Admin 里显示名 | `ci-bootstrap` |
| `CLAUDE_TAP_IMAGE` | claw-tap 镜像 | ACR `passionke/claw-tap:latest`（见 `env.production.example`） |

### Langfuse OTEL（CI 宿主机已部署 Langfuse 时）

在 **Settings → CI/CD → Variables** 配置；`render-env-from-ci.sh` 写入根 `.env`，`gateway.sh pool-up` 同步到 `pool-daemon.env`。详见 [`docs/langfuse-otel.md`](../../docs/langfuse-otel.md)。

| Key | Masked | 说明 |
|-----|--------|------|
| `LANGFUSE_PUBLIC_KEY` | **是** | Langfuse Project → API Keys |
| `LANGFUSE_SECRET_KEY` | **是** | 同上 |
| `CLAW_OTEL_ENABLED` | 否 | 建议 `1`；仅设 key 时 render 默认 `1` |
| `LANGFUSE_BASE_URL` | 否 | 94 CI：`http://10.22.28.94:8090`；未设时 render 用 `http://${CLAW_POOL_ADVERTISE_HOST}:8090` |
| `CLAW_OTEL_LOG_PROMPTS` | 否 | `1` 记录 prompt（默认）；`0` 关闭 |

双节点 `ci-cluster-dual-deploy.sh` 会把上述变量从 node A `.env` 转发到 `.env.ci-node-b`。

也可用通用名（二选一，bootstrap 优先读 `CLAW_BOOTSTRAP_*`）：

| Key | 等价于 |
|-----|--------|
| `OPENAI_API_KEY` | `CLAW_BOOTSTRAP_LLM_API_KEY` |
| `UPSTREAM_OPENAI_BASE_URL` / `OPENAI_BASE_URL` | `CLAW_BOOTSTRAP_LLM_BASE_URL` |
| `OPENAI_MODEL` | `CLAW_BOOTSTRAP_LLM_MODEL_NAME` |

## 3. 已在 `.gitlab-ci.yml` 写死（一般不用改 UI）

| Key | sunmax-i5 当前值 |
|-----|------------------|
| `CLAW_DEPLOY_PROFILE` | `production` |
| `CLAW_CONTAINER_RUNTIME` | `docker` |
| `CLAW_IMAGE_PREFIX` | `local` |
| `CLAW_RELEASE_SKIP_PULL` | `1` |
| `CLAW_USE_CN_CRATES_MIRROR` | `1`（cargo → rsproxy.cn sparse index） |
| `CLAW_USE_CN_RUST_MIRROR` | `1`（rustup/clippy/std → USTC rust-static） |
| `CLAW_POOL_ADVERTISE_HOST` | `10.22.28.94` |
| `CLAW_CLUSTER_ID` | `sunmi-ci-01` |
| `CLAW_POOL_ID` | `pool-sunmi-ci-01` |
| `CLAW_AUTO_BOOTSTRAP` | `1` |

换机器时：在 UI 覆盖 `CLAW_POOL_ADVERTISE_HOST` / `CLAW_CLUSTER_ID` / `CLAW_POOL_ID`，或改 `.gitlab-ci.yml` 里 `ci_production_docker` 块。

## 4. GitLab 添加步骤

1. 打开 `http://code.sunmi.com/minidata/claw-code` → **Settings** → **CI/CD** → **Variables** → **Add variable**
2. 添加 `CLAW_BOOTSTRAP_LLM_API_KEY`：勾选 **Mask variable**（仅此一项建议 Mask）
3. 添加 `CLAW_BOOTSTRAP_LLM_BASE_URL`：**不要 Mask**（URL 含 `:` `/` 时 Mask 会导致变量无法注入 job）
4. （可选）`CLAW_BOOTSTRAP_LLM_MODEL_NAME`：**不要 Mask**
5. **不要**勾选 **Protected**，除非已在 **Settings → Repository → Protected branches** 把 `main` 设为 protected；否则 deploy job 读不到变量（日志里会显示 `present … all no`）
6. **Environment scope** 保持 `*`（All）
7. **push 任意分支**（含 `main`、`proj_id` 等）自动触发：**build:release-images** → **deploy:release** → `admin-solve-e2e.sh`；job 检出 **当前 pipeline 提交**（不再强制 `main`）。镜像 tag：`main` → `release-<short_sha>`；其它分支 → `release-<ref_slug>-<short_sha>`（见 `deploy/stack/lib/ci-sync-worktree.sh`）。

变量名必须**完全一致**（区分大小写），不要用 `OPENAI_API_KEY` 代替除非你确认 job 日志里 `OPENAI_API_KEY=yes`。

## 5. 验收

`deploy:release` 日志应出现：

```
==> bootstrap PUT /v1/gateway/global-settings/active-llm-config
clawTap registered in Admin: host=claw-claude-tap ...
gateway clawTap ready (/readyz attempt …)
```

随后 `gateway.sh verify` → `admin-solve-e2e.sh`（两轮）→ `poll status=succeeded`。

**不覆盖集群：** CI 在 `deploy:release` 末尾跑 **`ci-cluster-dual-deploy.sh`**（同机双 gateway + 共享 PG + `cluster-verify`）。预发/生产多机另跑 `gateway.sh cluster-verify`（见 `deploy/stack/docs/cluster-deploy-verify.md`）。

**CI 盯盘 / glab**：见 `deploy/stack/docs/gitlab-cli.md`。

## 6. Runner 工作区

CI 使用 `GIT_CLEAN_FLAGS=-ffd`（不用 `-x`），避免 `git clean` 删除 root/uid1000 拥有的运行时目录导致 checkout 失败。下列路径须在 **`.gitignore`**（被 ignore 后 clean 会跳过）：

- `deploy/stack/claw-postgres-data/`
- `deploy/stack/claw-workspace/`、`deploy/stack/claw-workspace-*/`（含 `claw-workspace-ci-b`）
- `deploy/stack/claw-logs-*/`、`.claw-pool-rpc-*/`

PG 数据在 runner 上跨 pipeline 保留。

## 7. 每周磁盘清理（`maintenance:disk-prune`）

Job 定义见 `.gitlab-ci.yml`；脚本：`deploy/stack/lib/ci-disk-prune.sh`。

**清理范围**（不删 PG / workspace / 正在跑的 `:local` 镜像）：

- `gateway.sh clean --debug-only`（`rust/target` debug + `.linux-artifacts`）
- `docker image prune -f`（悬空层）
- `docker builder prune`（默认保留 7 天内 cache，可用 `CLAW_CI_BUILDER_PRUNE_UNTIL_HOURS` 覆盖）
- 各 `local/claw-*` 仓库只保留最新 **15** 个 `release-*` tag（`CLAW_CI_KEEP_RELEASE_TAGS` 可改）

### 7.1 定时执行（推荐）

1. GitLab → **Build** → **Pipeline schedules** → **New schedule**
2. **Description**：`weekly-disk-prune`
3. **Interval pattern**（cron）：`0 4 * * 0`（每周日 04:00，按 runner 时区）
4. **Target branch**：`main`（或你希望跑清理脚本的分支）
5. 保存后 GitLab 会按 cron 触发 pipeline；`CI_PIPELINE_SOURCE=schedule`，**只跑** `maintenance:disk-prune`，不跑 build/deploy。

### 7.2 手动执行

**Build** → **Pipelines** → **Run pipeline**：

- **Branch**：`main`
- **Variables**：`CLAW_DISK_PRUNE` = `1`
- Run → 同样只执行 `maintenance:disk-prune`。

本机 runner 上也可直接跑（不经过 GitLab）：

```bash
cd /path/to/claw-code
./deploy/stack/lib/ci-disk-prune.sh
```

## 8. 排查闭环（glab + runner）

- **glab 安装、登录、job trace、盯 pipeline**：[`deploy/stack/docs/gitlab-cli.md`](gitlab-cli.md)
- deploy 失败关键字：[`deploy/stack/docs/gitlab-ci-troubleshoot.md`](gitlab-ci-troubleshoot.md)
- 盯 pipeline：`./deploy/stack/lib/ci-watch-pipeline.sh main`
- push 触发 CI：`git push sunmi main`

## 9. 参考

- 变量模板：`deploy/stack/env.ci.example`
- 生成脚本：`deploy/stack/lib/render-env-from-ci.sh`
- 启动 bootstrap：`deploy/stack/lib/bootstrap-runtime.sh`
- 磁盘清理：`deploy/stack/lib/ci-disk-prune.sh`

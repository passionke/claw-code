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

随后 `admin-solve-e2e.sh` → `poll status=succeeded`。

## 6. Runner 工作区

CI 使用 `GIT_CLEAN_FLAGS=-ffd`（不用 `-x`），避免 `git clean` 删除 root 拥有的 `deploy/stack/claw-postgres-data/` 导致 checkout 失败。PG 数据在 runner 上跨 pipeline 保留。

## 7. 参考

- 变量模板：`deploy/stack/env.ci.example`
- 生成脚本：`deploy/stack/lib/render-env-from-ci.sh`
- 启动 bootstrap：`deploy/stack/lib/bootstrap-runtime.sh`

# GitHub Actions CI 环境变量（passionke/claw-code）

在仓库 **Settings → Secrets and variables → Actions** 配置；job 跑 `./deploy/stack/lib/render-env-from-ci.sh` 生成仓库根 `.env`，**不要在 runner 上手写 `.env`**。

**触发方式**：**Actions → claw-ci-deploy → Run workflow**（手工，不随 push 自动跑）。

**Runner**：自托管，标签 `contabo-sg`（`vmi3350843`），宿主机 **62.72.45.75**。

Author: kejiqing

## 1. 必须在 GitHub 配置的 Secrets / Variables

| Key | 类型 | 说明 | 示例 |
|-----|------|------|------|
| `CLAW_BOOTSTRAP_LLM_API_KEY` | **Secret** | LLM API Key；`up` 时写入 PG active LLM | `sk-...` |
| `CLAW_BOOTSTRAP_LLM_BASE_URL` | **Variable** | OpenAI 兼容 base URL，**须含 `/v1`**；URL **不要**放 Secret | `https://api.deepseek.com/v1` |

`release` job 在 deploy 阶段设 `CLAW_CI_REQUIRE_LLM_BOOTSTRAP=1`，缺上述两项时 `render-env-from-ci.sh` 直接失败。

**原则**：代码只进 GitHub；部署由 **Actions → claw-ci-deploy → Run workflow** 驱动 `contabo-sg` runner，**不要** rsync/手改服务器上的仓库。

## 2. 建议配置的 Variables（可选）

| Key | 说明 | 默认 |
|-----|------|------|
| `CLAW_BOOTSTRAP_LLM_MODEL_NAME` | 模型 id | `gpt-4o-mini` |
| `CLAW_BOOTSTRAP_LLM_NAME` | Admin 里显示名 | `github-ci-llm` |
| `CLAUDE_TAP_IMAGE` | claw-tap 镜像 | ACR `passionke/claw-tap:latest` |

### Langfuse OTEL（可选）

| Key | 类型 | 说明 |
|-----|------|------|
| `LANGFUSE_PUBLIC_KEY` | Secret | Langfuse Project → API Keys |
| `LANGFUSE_SECRET_KEY` | Secret | 同上 |
| `CLAW_OTEL_ENABLED` | Variable | 建议 `1` |
| `LANGFUSE_BASE_URL` | Variable | 默认 `http://62.72.45.75:8090` |
| `CLAW_OTEL_LOG_PROMPTS` | Variable | `1` 记录 prompt；`0` 关闭 |

## 3. 已在 workflow 写死（一般不用改）

| Key | 62.72.45.75 当前值 |
|-----|-------------------|
| `CLAW_POOL_ADVERTISE_HOST` | `62.72.45.75` |
| `CLAW_CLUSTER_ID` | `github-ci-01` |
| `CLAW_POOL_ID` | `pool-github-ci-01` |
| `CLAW_CI_NODE_B_POOL_ID` | `pool-github-ci-02` |
| `CLAW_DEPLOY_PROFILE` | `production` |
| `CLAW_CONTAINER_RUNTIME` | `docker` |
| `CLAW_IMAGE_PREFIX` | `local` |
| `CLAW_RELEASE_SKIP_PULL` | `1` |
| `CLAW_USE_CN_CRATES_MIRROR` | `0`（SG 机房；Sunmi 国内 CI 用 `1`） |
| `CLAW_USE_CN_RUST_MIRROR` | `0`（SG 机房；Sunmi 国内 CI 用 `1`） |
| `CONTAINER_BASE_REGISTRY` | `docker.1ms.run` |
| `CLAUDE_TAP_IMAGE` | `ghcr.io/passionke/claude-tap:latest`（SG；Sunmi 国内用 ACR） |

换机器时：改 `.github/workflows/claw-ci-deploy.yml` 里 `env:` 块，或用 repo Variables 覆盖。

## 4. 对外端口（防火墙）

| 服务 | 端口 | 对外 |
|------|------|------|
| Admin `/admin` | `18765` | **已开** |
| clawTap Live | `3000` | **已开** |
| Gateway API | `18088` | **暂不开**（仅本机 e2e / 内网） |

`render-env-from-ci.sh` 默认：`GATEWAY_PLAYGROUND_HOST_PORT=18765`，`CLAUDE_TAP_PUBLISH_LIVE=0.0.0.0:3000:3000`。

## 5. 安装 self-hosted runner（首次）

在 **62.72.45.75** 上以 root 或专用用户执行（token 从 GitHub UI 获取，24h 有效）：

```bash
# GitHub → Settings → Actions → Runners → New self-hosted runner → Linux x64
mkdir -p /opt/actions-runner && cd /opt/actions-runner
curl -fsSL -o actions-runner.tar.gz -L \
  https://github.com/actions/runner/releases/download/v2.323.0/actions-runner-linux-x64-2.323.0.tar.gz
tar xzf actions-runner.tar.gz
./config.sh --url https://github.com/passionke/claw-code --token <REGISTRATION_TOKEN> \
  --labels contabo-sg --name vmi3350843 --unattended
./svc.sh install && ./svc.sh start
```

验收：`./svc.sh status` 显示 active；GitHub Runners 页显示 **Idle**。

## 6. 手工触发 deploy

1. 配好 Secrets（§1）
2. **Actions → claw-ci-deploy → Run workflow**
3. `ref`：默认 `main`；首次可勾 `skip_dual_deploy` 加快验收
4. 日志应出现：`render-env-from-ci.sh` → `gateway.sh build` → `up --release` → `verify` → `admin-solve-e2e`

**Workflow inputs**：

| Input | 说明 |
|-------|------|
| `ref` | 分支或 tag |
| `skip_dual_deploy` | 跳过 node B 双机验证 |
| `disk_prune_only` | 只跑磁盘清理 |

## 7. 与 Sunmi GitLab CI 对照

| | Sunmi GitLab | GitHub |
|--|--------------|--------|
| 触发 | push 任意分支 | **workflow_dispatch** |
| Runner 标签 | `claw-dev` | `contabo-sg` |
| 同步脚本 | `ci-sync-worktree.sh` | `ci-sync-worktree-github.sh` |
| 集群 id | `sunmi-ci-01` | `github-ci-01` |
| 宿主机 | `10.22.28.94` | `62.72.45.75` |

## 8. 参考

- 变量模板：`deploy/stack/env.ci.github.example`
- 生成脚本：`deploy/stack/lib/render-env-from-ci.sh`
- Workflow：`.github/workflows/claw-ci-deploy.yml`
- Sunmi 对照：`deploy/stack/docs/gitlab-ci-variables.md`

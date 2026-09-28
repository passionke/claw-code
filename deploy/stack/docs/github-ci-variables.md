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
| `CLAW_BOOTSTRAP_LLM_MODEL_NAME` | 模型 id | **`deepseek-v4-flash`**（DeepSeek base URL 时）；勿用默认 `gpt-4o-mini` |
| `CLAW_BOOTSTRAP_LLM_NAME` | Admin 里显示名 | `github-ci-llm` |
| `CLAUDE_TAP_IMAGE` | claw-tap 镜像 | ACR `passionke/claw-tap:latest` |

### claw-code-image（release 打 tag 自动跑）

Workflow：`.github/workflows/claw-code-image.yaml`（push `release-v*` tag）。

| Key | 类型 | 说明 |
|-----|------|------|
| `CONTAINER_BASE_REGISTRY` | Variable | 可选；未设 → `docker.io`（github-hosted 编译推荐）。 |

**release 编译 cache 验收**（`linux-compile-once` job 日志）：

1. `Cache restored` 或 partial restore（非 `Cache not found`）— lock 不变且非首次 warm
2. Post cache 无 `Permission denied` / `Failed to save`
3. `ci-cache ownership ok for uid=...`
4. sccache `Cache hits rate` > 0（lock 不变、第二次 release 起）

本地脚本（需 `gh`）：

```bash
./deploy/stack/lib/ci-verify-linux-compile-cache.sh <run-id>
# gh run list --workflow claw-code-image.yaml --limit 3
```

**GHCR 预构建 compile 镜像**：同 workflow 的 `rust-compile-image` job 推送 `ghcr.io/<owner>/claw-rust-compile:1.88-bookworm`；`linux-compile-once` 优先 pull，避免每次 apt 装 mold/sccache。

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

## 8. mirror-to-acr：香港阿里云 → 杭州 ACR（公网，无 VPN）

**路径**（Author: kejiqing）：

1. **github-hosted (`ubuntu-latest`)**：编译并推 GHCR  
2. **self-hosted `aliyun-hk`**（`cn-hongkong`，`8.210.177.9`）：`skopeo copy --format v2s2` 从 GHCR 拉到 ACR  

个人版 ACR 不接受 OCI empty layer；必须用 **skopeo v2s2**，不要 `docker pull/tag/push`。凭证仍在 Environment（`vars.ACR_GITHUB_ENVIRONMENT`，默认 `claw-acr`）：`ACR_USERNAME` / `ACR_PASSWORD`；registry 前缀用 repo Variable **`ACR_REGISTRY`**。

`mirror-to-acr`：`runs-on: [self-hosted, aliyun-hk]`，`max-parallel: 1`（机器约 1G 内存）。

### 安装 / 验收 aliyun-hk runner

```bash
# 在香港机（user runner，已加 docker 组）
cd /home/runner/actions-runner
# GitHub → Settings → Actions → Runners → New self-hosted runner → token
sudo -u runner ./config.sh --url https://github.com/passionke/claw-code \
  --token <REGISTRATION_TOKEN> --name aliyun-hk --labels aliyun-hk --unattended
./svc.sh install runner && ./svc.sh start
```

验收：`systemctl status actions.runner.passionke-claw-code.aliyun-hk` active；GitHub Runners 页 **aliyun-hk** Idle；本机：

```bash
curl -sS -o /dev/null -w "%{http_code} tls=%{time_appconnect}\n" \
  --connect-timeout 15 \
  https://crpi-cf9vxpq3n8or17mw.cn-hangzhou.personal.cr.aliyuncs.com/v2/
# 期望 401 且 tls 远小于 1s
```

> 旧 SG VPN 路由（`ACR_MIRROR_VPN_*` / `ci-acr-vpn-route.sh`）已不再用于 `claw-code-image` mirror；deploy 仍可走 `contabo-sg`（§5）。

## 9. 参考

- 变量模板：`deploy/stack/env.ci.github.example`
- 生成脚本：`deploy/stack/lib/render-env-from-ci.sh`
- Workflow：`.github/workflows/claw-ci-deploy.yml`、`.github/workflows/claw-code-image.yaml`
- release 编译 cache 验收：`deploy/stack/lib/ci-verify-linux-compile-cache.sh`
- Sunmi 对照：`deploy/stack/docs/gitlab-ci-variables.md`

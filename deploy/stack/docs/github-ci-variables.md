# GitHub Actions CI 环境变量（passionke/claw-code）

在仓库 **Settings → Secrets and variables → Actions** 配置；job 跑 `./deploy/stack/lib/render-env-from-ci.sh` 生成仓库根 `.env`，**不要在 runner 上手写 `.env`**。

**镜像构建 Runner**：自托管 **`home-ubt`**（`passionkeosxubt` / **10.8.0.21**）— 本机编译 + skopeo 直推 ACR，**不再**走 GHCR / aliyun-hk / contabo-sg。

**Deploy Runner**：`claw-ci-deploy` 仍标 `contabo-sg`；该 runner 已下线，deploy 流水线需另择宿主后再开。

Author: kejiqing

## 1. 必须在 GitHub 配置的 Secrets / Variables

| Key | 类型 | 说明 | 示例 |
|-----|------|------|------|
| `CLAW_BOOTSTRAP_LLM_API_KEY` | **Secret** | LLM API Key；`up` 时写入 PG active LLM | `sk-...` |
| `CLAW_BOOTSTRAP_LLM_BASE_URL` | **Variable** | OpenAI 兼容 base URL，**须含 `/v1`**；URL **不要**放 Secret | `https://api.deepseek.com/v1` |
| `ACR_USERNAME` / `ACR_PASSWORD` | **Secret** | 杭州 ACR 登录 | — |
| `ACR_REGISTRY` | **Variable** | ACR 前缀（含 namespace） | `crpi-….cn-hangzhou.personal.cr.aliyuncs.com/passionke` |

`release` job 在 deploy 阶段设 `CLAW_CI_REQUIRE_LLM_BOOTSTRAP=1`，缺上述 LLM 两项时 `render-env-from-ci.sh` 直接失败。

**原则**：代码只进 GitHub；镜像由 **home-ubt** 打包直推 ACR。

## 2. 建议配置的 Variables（可选）

| Key | 说明 | 默认 |
|-----|------|------|
| `CLAW_BOOTSTRAP_LLM_MODEL_NAME` | 模型 id | **`deepseek-v4-flash`**（DeepSeek base URL 时） |
| `CLAW_BOOTSTRAP_LLM_NAME` | Admin 里显示名 | `github-ci-llm` |
| `CLAUDE_TAP_IMAGE` | claw-tap 镜像 | ACR `passionke/claw-tap:latest` |
| `CONTAINER_BASE_REGISTRY` | 基础镜像 registry hostname | home-ubt 上 job 默认 `docker.1ms.run` |
| `ACR_GITHUB_ENVIRONMENT` | Environment 名（挂 ACR 相关 vars） | `dockerhub` 或 `claw-acr` |

### claw-code-image（release 打 tag 自动跑）

Workflow：`.github/workflows/claw-code-image.yaml`（push `release-v*` tag）。

链路：**home-ubt** 编译 → 本地 docker build → `ci-push-acr-skopeo.sh`（v2s2）→ ACR。不经 GHCR。

## 3. 已在 workflow 写死（一般不用改）

| Key | home-ubt / deploy 备注 |
|-----|------------------------|
| `CLAW_USE_CN_CRATES_MIRROR` | image jobs：`1`（国内机） |
| `CLAW_USE_CN_RUST_MIRROR` | image jobs：`1` |
| `CLAW_USE_CN_APT_MIRROR` | image jobs：`1` |
| `CLAW_IMAGE_PREFIX` | deploy 示例仍为 `local`（contabo 时代） |

## 4. 对外端口（防火墙）— 仅 deploy 机相关

| 服务 | 端口 | 对外 |
|------|------|------|
| Admin `/admin` | `18765` | 视 deploy 宿主 |
| clawTap Live | `3000` | 视 deploy 宿主 |
| Gateway API | `18088` | 通常仅内网 |

## 5. 安装 self-hosted runner（home-ubt / 10.8.0.21）

包可从本机 Nora raw 拉（勿直连 GitHub releases）：

```bash
# https://nora.home.passionke.top/raw/github/actions-runner/actions-runner-linux-x64-2.337.0.tar.gz
mkdir -p ~/actions-runner && cd ~/actions-runner
curl -fL -o ../actions-runner-linux-x64-2.337.0.tar.gz \
  https://nora.home.passionke.top/raw/github/actions-runner/actions-runner-linux-x64-2.337.0.tar.gz
tar xzf ../actions-runner-linux-x64-2.337.0.tar.gz
# GitHub → Settings → Actions → Runners → New self-hosted runner → token
./config.sh --url https://github.com/passionke/claw-code --token <REGISTRATION_TOKEN> \
  --name passionkeosxubt --labels home-ubt --unattended --replace
sudo ./svc.sh install passionke && sudo ./svc.sh start
```

依赖：`docker`（用户在 docker 组）、`skopeo`（`apt install skopeo`）。

验收：`systemctl status actions.runner.passionke-claw-code.passionkeosxubt` active；GitHub Runners 页 **passionkeosxubt** Idle。

镜像 job 结束后会跑 `ci-home-ubt-post-image-cleanup.sh`：清 docker 本地 tag / dangling / build cache、linux-artifacts、`rust/target`、claw-ci-artifacts；**保留** `.ci-cache`（sccache + crates，下次编译加速）。

CI 编译把 `CARGO_TARGET_DIR` 指到 `.linux-artifacts`（不是默认 `rust/target`）；`linux-compile.sh` 在编完后立刻只留 `claw` / `http-gateway-rs` 两个二进制，删掉 deps/build 等 target 残骸，避免 GB 级残留。

## 6. 手工触发 branch worker / deploy

1. **Actions → claw-code-branch-worker → Run workflow**（分支 worker → ACR）
2. **Actions → claw-ci-deploy**：当前无 `contabo-sg` runner，勿指望能跑通，直到另择 deploy 宿主

## 7. 与 Sunmi GitLab CI 对照

| | Sunmi GitLab | GitHub image |
|--|--------------|--------------|
| 触发 | push 任意分支 | tag / workflow_dispatch |
| Runner 标签 | `claw-dev` | `home-ubt` |
| 镜像目标 | 视 Sunmi | **ACR only** |
| 宿主机 | `10.22.28.94` | `10.8.0.21` |

## 8. ACR 推送（skopeo v2s2）

个人版 ACR 不接受 OCI empty layer；必须用 **skopeo `--format v2s2`**（`deploy/stack/lib/ci-push-acr-skopeo.sh`），不要只靠 `docker push`。

凭证：repo Secrets `ACR_USERNAME` / `ACR_PASSWORD`；前缀 Variable **`ACR_REGISTRY`**。

已下线：`aliyun-hk`（弱）、`vmi3350843` / `contabo-sg`（网络差）。

## 9. 参考

- 变量模板：`deploy/stack/env.ci.github.example`
- 生成脚本：`deploy/stack/lib/render-env-from-ci.sh`
- Workflow：`.github/workflows/claw-code-image.yaml`、`.github/workflows/claw-code-branch-worker.yaml`
- ACR 推送：`deploy/stack/lib/ci-push-acr-skopeo.sh`
- Nora raw runner 包：`https://nora.home.passionke.top/raw/github/actions-runner/`
- Sunmi 对照：`deploy/stack/docs/gitlab-ci-variables.md`

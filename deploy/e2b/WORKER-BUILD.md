# e2b Worker 空壳 + CLI 注入 — 唯一手册

Author: kejiqing

**自托管 e2b（10.8.0.x）worker 节点全是 `linux/amd64`。**

## 三件事

| 层 | 谁负责 | 做什么 |
|----|--------|--------|
| **1. 空壳镜像** | `deploy/pack/publish.sh worker-base` | 打 `claw-worker-base` / `relaxed`（**无** claw/neuro）进制品库 |
| **2. e2b 注册** | `publish.sh e2b-register` 或 Admin「制作模板」 | `from_image` 注册空壳；写 PG `templateId`/`buildId` |
| **3. CLI 版本** | `publish.sh cli-*` + Admin「Worker CLI 版本」 | 制品进库 → 钉死完整 ref+digest → create 时 platform worker.init 安装 |

**禁止**再对 claw 做 registry extract + debian COPY 双烤；**禁止**独立引擎 e2b 模板。

## 打包入口（唯一）

```bash
# 日常 Jenkins / 大版本 GHA self-hosted 都调这一支
RELEASE_TAG=v1.2.3 CLAW_IMAGE_PREFIX=nora.home.passionke.top/passionke \
  ./deploy/pack/publish.sh gateway|worker-base|cli|all

# 空壳注册到 e2b（需 CLAW_E2B_API_* + PG）
RELEASE_TAG=v1.2.3 ./deploy/pack/publish.sh e2b-register
```

契约见 [`deploy/SERVICES.md`](../SERVICES.md)。

## Gateway 启动

与以前相同：期望 `buildId` 与沙箱 pin 不一致才切换空壳；**CLI 版本**靠 reset worker 后新 create 时 inject，不因改 cliPins 杀健康沙箱。

## 验收

```bash
# 空壳无 claw
docker run --rm --entrypoint sh "$PREFIX/claw-worker-base:$TAG" -c '! command -v claw'

# 新沙箱内（inject 后）
command -v claw && claw --version
```

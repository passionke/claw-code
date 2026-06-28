# Sunmi GitLab CI 排查闭环（claw-code）

> **2026-06：** `:9944` pool / `host-pool-daemon.md` 已废弃。FC 503 查 `CLAW_FC_API_URL` 与 e2b 模板（`deploy/fc-sandbox/README.md`）。

Author: kejiqing

**glab 安装 / 登录 / 常用命令 / 盯 pipeline**：见 **[`gitlab-cli.md`](gitlab-cli.md)**（必读，避免「有一搭没一搭」改 CI）。

## 1. CI 契约（`.gitlab-ci.yml`）

Push 任意分支 → `build:release-images` → `deploy:release`：

1. `gateway.sh up --release`
2. `gateway.sh verify`
3. `admin-solve-e2e.sh` × 2
4. **`ci-cluster-dual-deploy.sh`** — node B + **`cluster-verify`** + **`ci-cluster-solve-e2e`**（独立 workspace、跨 gateway session、node B relaxed）

**验收**：`deploy:release` 全绿 = node A `admin-solve-e2e`×2 **且** node B solve + **A 建 session → B 续聊** 均 `succeeded`；Permission denied 说明 node B workspace 未 chown 1000（`ci-cluster-node-up` 应修）。

## 2. 端点

| 角色 | 地址 |
|------|------|
| GitLab UI | `http://code.sunmi.com/minidata/claw-code` |
| CI Runner | `http://10.22.28.94/` |
| Node A 网关 | `http://10.22.28.94:18088/healthz` |
| Node B 网关（CI） | `http://10.22.28.94:18089/healthz` |
| Pool strict | `http://10.22.28.94:9944/healthz/live-report` |
| Admin | `http://10.22.28.94:18765/` |

Runner 工作目录：`/home/gitlab-runner/builds/.../minidata/claw-code`（`GIT_CLEAN_FLAGS=-ffd`，保留 `deploy/stack/claw-postgres-data`）。

## 3. 标准排查顺序（deploy 失败）

1. **`glab api …/jobs/JOB_ID/trace`** 或 `./deploy/stack/lib/ci-watch-pipeline.sh <branch>`（见 `gitlab-cli.md`）。
2. **对应 commit** 是否含预期修复（`git log sunmi/main -3`）。
3. **`build:release-images`** 是否 success（失败看 trace 里 `gateway.sh build`）。
4. **`deploy:release` 卡在哪一步**：
   - `Postgres ready` / `waiting gateway HTTP`
   - `gateway deploy failure diagnostics` → `docker logs claw-gateway-rs`
   - `admin-solve-e2e` / `poll status=succeeded`
   - **`ci-cluster-dual-deploy`** / **`CLUSTER VERIFY`**
5. **Runner 探测**（同网段）：见 `gitlab-cli.md` §6。

### deploy 日志关键字

| 关键字 | 含义 |
|--------|------|
| `Restarting (1)` | 网关容器崩溃循环 → `docker logs claw-gateway-rs` |
| `schema migration failed` | PG 迁移；查 SQL 与 `session_db` migrate |
| `503` / FC unavailable | 查 `CLAW_FC_API_URL`、API key、e2b 健康；见 `deploy/fc-sandbox/README.md` |
| `no such service: postgres` | node B 误对 gateway-only compose 起 postgres → 应复用 A 的 `claw-gateway-postgres` |
| `POOL_B: unbound variable` | `ci-cluster-dual-deploy.sh` 未定义 pool id（已修，见 `gitlab-cli.md` §7） |
| `CLUSTER VERIFY FAIL` | `claw-cluster-verify.sh`；查 `claw_pool` 僵尸行 / 各 gateway `/v1/pools` |
| `clawTap clusterId sunmi-ci-01 does not match configured sunmi-ci-02` | 同机 CI 只有一个 clawTap；node B 须与 node A **同一 `CLAW_CLUSTER_ID`** |
| CI `pool-up` 卡在 `cleaning legacy dual-pool listener on :9954` 很久 | 94 上 **dev-stable** 占用 `:9954`（`claw-sandbox`）；旧逻辑误杀并 `docker pull alpine`。已修复：`pool-health.sh` 识别现代 sandbox 后跳过；升级 `langfuse` 分支后重跑 deploy |
| `failed to remove deploy/stack/claw-workspace-ci-b/... Permission denied` | checkout 阶段 `git clean`；目录须进 `.gitignore`（`claw-workspace-*/`），见 `gitlab-ci-variables.md` |

## 4. Agent 闭环

1. **证据**：job trace 行号 + 脚本路径（禁止无 trace 的下结论）。
2. 最小 diff；Rust 改动跑 `cargo fmt` / `clippy` / `test`。
3. `git push sunmi <branch>`。
4. `./deploy/stack/lib/ci-watch-pipeline.sh <branch>`。
5. 仍失败 → 新 job id，重复 1–4；**不改** `.gitlab-ci.yml` 阶段契约，除非用户明确要求。

## 5. 相关文档

- **glab / 盯盘 / job 链接**：[`gitlab-cli.md`](gitlab-cli.md)
- CI 变量：[`gitlab-ci-variables.md`](gitlab-ci-variables.md)
- 集群验收：[`cluster-deploy-verify.md`](cluster-deploy-verify.md)
- FC / Admin 503：[`deploy/fc-sandbox/README.md`](../../fc-sandbox/README.md)、[`docs/deploy-ops-truth.md`](../../../docs/deploy-ops-truth.md)
- 部署总览：[`../README.md`](../README.md)

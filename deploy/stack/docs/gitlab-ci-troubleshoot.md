# Sunmi GitLab CI 排查闭环（claw-code）

Author: kejiqing

固定 **CI 契约**（`.gitlab-ci.yml` 不轻易改）：`push 任意分支` → `build:release-images` → `deploy:release` → `admin-solve-e2e.sh`。  
Agent / 开发在**同一 pipeline 形态**下迭代：改代码 → push `proj_id`（或目标分支）→ `glab` 看 job → 修到 `deploy` + e2e 绿。

## 1. 端点

| 角色 | 地址 |
|------|------|
| GitLab（代码 / pipeline UI） | `http://code.sunmi.com/minidata/claw-code` |
| **CI Runner 宿主机**（跑 job 的机器） | **`http://10.22.28.94/`**（LAN；与 `CLAW_POOL_ADVERTISE_HOST` 一致） |
| 网关 healthz（runner 上） | `http://10.22.28.94:18088/healthz` |
| Pool live-report（runner 上） | `http://10.22.28.94:9944/healthz/live-report` |
| Playground / Admin | `http://10.22.28.94:18765/` |

Runner 工作目录（持久）：`/home/gitlab-runner/builds/.../minidata/claw-code`（`GIT_CLEAN_FLAGS=-ffd`，**保留** `deploy/stack/claw-postgres-data`）。

## 2. 本机 CLI（`glab`，已登录 `code.sunmi.com`）

```bash
# 最近 pipeline
glab ci list --repo minidata/claw-code --per-page 10

# 某条 pipeline 的 job 清单（把 PIPELINE_ID 换成实际数字，如 91547）
glab api "projects/minidata%2Fclaw-code/pipelines/PIPELINE_ID/jobs" \
  | python3 -c "import json,sys; [print(j['status'], j['stage'], j['name'], j['id']) for j in json.load(sys.stdin)]"

# 拉 deploy job 完整日志（把 JOB_ID 换成实际数字，如 214201）
glab api "projects/minidata%2Fclaw-code/jobs/JOB_ID/trace" | tail -80

# 盯当前分支最新 pipeline 直到结束（仓库脚本）
./deploy/stack/lib/ci-watch-pipeline.sh proj_id
```

## 3. 标准排查顺序（deploy 失败）

1. **看 pipeline 对应 commit**（`proj_id` 上应为修复提交，而非仅 CI 触发改动）。
2. **`build:release-images` 是否 success**（~7min；失败先看 `deploy/stack/.build.log` 或 job trace 里的 `gateway.sh build`）。
3. **`deploy:release` 日志关键字**：
   - `Postgres ready` — PG 正常
   - `waiting gateway HTTP` — 网关未监听 `:18088`
   - `gateway deploy failure diagnostics` — 脚本已 dump `docker ps` + `docker logs claw-gateway-rs`
   - `Restarting (1)` — 容器进程 **exit 1 崩溃循环**（查 `docker logs` 里的 `http-gateway-rs:` / `migration` / `CLAW_`）
   - `admin-solve-e2e` / `status=succeeded` — 最终验收
4. **Runner 上快速探测**（同网段可 curl）：
   ```bash
   curl -fsS "http://10.22.28.94:18088/healthz" | head -c 200
   curl -fsS "http://10.22.28.94:9944/healthz/live-report" | head -c 200
   ```
5. **典型根因示例**（job 214206 / `f531fcc9`）：
   - `docker logs`：`schema migration failed: column "proj_id" does not exist`
   - `failed SQL: UPDATE project_config SET proj_id = ds_id ...`（前面的 `ALTER ADD COLUMN` 被跳过）
   - 修复：`run_sql_migration_file` 须剥掉段首 `--` 注释行，不能整段 `starts_with("--")` 跳过。
6. **已知坑**（勿重查错方向）：
   - gateway 与 pool-daemon **不要并发** `GatewaySessionDb::migrate()`（`up.sh` 先等 healthz，pool-daemon 用 `open_without_migrate`）。
   - macOS 本地 pool 用 launchd；**Linux CI runner 用 systemd/nohup**（见 `host-pool-daemon.md`）。
   - 只 curl healthz **不算** Admin solve 通过；验收必须 `admin-solve-e2e.sh` 多轮 `succeeded`。

## 4. Agent 闭环（责任边界）

1. 读 `glab` job trace / 代码定位根因（要有证据：日志行或代码路径）。
2. 最小修复 diff + `cargo fmt` / `clippy` / `test`（Rust 改动时）。
3. `git push sunmi <branch>`（触发 CI；pre-push hook 跑相关测试）。
4. `./deploy/stack/lib/ci-watch-pipeline.sh <branch>` 等到 `deploy:release` 结果。
5. 仍失败 → 用新 job id 重复 1–4，**不改** `.gitlab-ci.yml` 阶段契约，除非用户明确要求。

## 5. 相关文档

- CI 变量：`deploy/stack/docs/gitlab-ci-variables.md`
- Pool / Admin 503：`deploy/stack/docs/host-pool-daemon.md`
- 部署总览：`deploy/stack/README.md`

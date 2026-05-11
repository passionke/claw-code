# Rust Gateway 快速使用说明

这份文档给你一个最短路径：**怎么启动、怎么测通、怎么停掉**。

配套文档：
- 接口清单：`docs/http-gateway-rs-api.md`
- 文档维护实践：`docs/http-gateway-rs-docs-practice.md`

## 一、直接本地运行（不走容器）

在仓库根目录执行：

```bash
cd rust
cargo run -p http-gateway-rs
```

默认监听：`0.0.0.0:8080`

可选环境变量：

- `CLAW_HTTP_ADDR`：监听地址，例 `127.0.0.1:8088`
- `CLAW_BIN`：`claw` 可执行文件路径
- `CLAW_WORK_ROOT`：工作目录，默认 `/tmp/claw-workspace`
- `CLAW_CONFIG_FILE`：项目 `.claw.json` 路径（用于合并 `mcpServers`）；不设则不从文件读 MCP
- `CLAW_PROJECT_CONFIG_ROOT`：可选，显式指定 `ConfigLoader` 目录；不设则用 `CLAW_CONFIG_FILE` 的父目录
- `CLAW_TIMEOUT_SECONDS`：`solve` 超时秒数
- `CLAW_DEFAULT_HTTP_MCP_NAME`：默认 HTTP MCP 名称（例 `claude-tap`）
- `CLAW_DEFAULT_HTTP_MCP_URL`：默认 HTTP MCP URL（例 `http://127.0.0.1:9091/mcp`）
- `CLAW_DEFAULT_HTTP_MCP_TRANSPORT`：`http` 或 `sse`

**网关可观测性（本仓库 `http-gateway-rs` 专用）**

- `CLAW_GATEWAY_LOG_DIR`：可选；**JSON 行**按天滚动写入该目录（文件前缀 `http-gateway`）。不设时默认 **`$CLAW_WORK_ROOT/.claw-gateway-logs/`**（进程启动前会 `mkdir`）。
- `CLAW_GATEWAY_FILE_LOG`：设为 `0` / `false` / `off` 时**只打 stdout**，不写上述目录（适合本地短时调试）。
- 结构化字段与 `target`：`claw_gateway_orchestration`（编排）、`claw_gateway_pool`（`docker run/exec`）、`claw_gateway_solve_pool`（池化 solve 编排）、`claw_gateway_solve`（worker 子进程 stderr 行）。可用 `RUST_LOG=claw_gateway_pool=debug` 等收窄噪声。
- 与全进程共用：`CLAW_LOG_LEVEL`。`CLAW_LOG_FORMAT`：在**未启用**上述文件 sink 时控制 stdout；**一旦**写 `CLAW_GATEWAY_LOG_DIR`（或默认目录）且未 `CLAW_GATEWAY_FILE_LOG=off`，**stdout 与文件均为 JSON**（`tracing-subscriber` 双层的类型限制）。

示例：

```bash
cd rust
CLAW_HTTP_ADDR=127.0.0.1:8088 \
CLAW_BIN=/Users/$USER/work/claw-code/rust/target/debug/claw \
CLAW_DEFAULT_HTTP_MCP_NAME=claude-tap \
CLAW_DEFAULT_HTTP_MCP_URL=http://127.0.0.1:9091/mcp \
CLAW_DEFAULT_HTTP_MCP_TRANSPORT=http \
cargo run -p http-gateway-rs
```

## 二、Podman 镜像运行（推荐）

**以 `deploy/podman/README.md` 为准**：根目录 `.env`、`./deploy/podman/build.sh`、worker 镜像、`./deploy/podman/up.sh` 一条链；不要自己拼 `podman compose` 参数。

摘要：

```bash
cp .env.example .env   # 编辑：PODMAN_HOST_SOCK、OPENAI_*、GATEWAY_HOST_PORT 等
./deploy/podman/build.sh
# 按 README 构建 claw-gateway-worker:local
./deploy/podman/up.sh
./deploy/podman/check-connectivity.sh
```

需要 claude-tap 时用 `./deploy/podman/start-with-tap.sh` / `stop-with-tap.sh`。停止 compose 栈：`./deploy/podman/down.sh`。

## 三、接口怎么调

### 健康检查

```bash
curl -sS "http://127.0.0.1:18088/healthz"
```

### 同步调用

```bash
curl -sS -X POST "http://127.0.0.1:18088/v1/solve" \
  -H "Content-Type: application/json" \
  -d '{
    "dsId": 1,
    "userPrompt": "给我一个简短总结"
  }'
```

若网关配置了非空的 `CLAW_ALLOWED_TOOLS`，请求里的 `allowedTools` 只能从中再选子集；要让模型用 shell、写文件、调 MCP，需全局白名单先包含这些名，再在 body 里列出，例如：

```bash
curl -sS -X POST "http://127.0.0.1:18088/v1/solve" \
  -H "Content-Type: application/json" \
  -d '{
    "dsId": 1,
    "userPrompt": "把任务说明里的内容整理成 .claw/skills/foo/SKILL.md 并自检",
    "allowedTools": [
      "read_file", "glob_search", "grep_search",
      "bash", "write_file", "edit_file", "MCP"
    ]
  }'
```

### 异步调用 + 轮询

```bash
curl -sS -X POST "http://127.0.0.1:18088/v1/solve_async" \
  -H "Content-Type: application/json" \
  -d '{"dsId":1,"userPrompt":"ping"}'
```

返回里拿到 `taskId` 后：

```bash
curl -sS "http://127.0.0.1:18088/v1/tasks/<taskId>"
```

仍在排队或运行中时，可按同一 `taskId` 取消：

```bash
curl -sS -X POST "http://127.0.0.1:18088/v1/tasks/<taskId>/cancel"
```

### 生成清洗后的最终报告

当 `/v1/tasks/<taskId>` 返回 `status=succeeded` 后，可调用报告接口：

```bash
curl -sS "http://127.0.0.1:18088/v1/biz_advice_report?task_id=<taskId>"
```

返回关键字段：

- `taskId`：原异步任务 ID
- `sourceRequestId`：原任务 requestId
- `sourceDsId`：原任务 dsId
- `sourceStatus`：原任务状态
- `reportText`：清洗后的最终报告文本
- `reportJson`：清洗后的结构化结果

### MCP 注入/查看

```bash
curl -sS -X POST "http://127.0.0.1:18088/v1/mcp/inject" \
  -H "Content-Type: application/json" \
  -d '{
    "dsId": 1,
    "mcpServers": {
      "demo": {
        "type": "http",
        "url": "http://127.0.0.1:9091/mcp"
      }
    },
    "replace": false
  }'
```

```bash
curl -sS "http://127.0.0.1:18088/v1/mcp/injected/1"
```

## 四、最常见问题

- **18088 打不开**
  - 检查 `podman ps` 是否有 `0.0.0.0:18088->8080/tcp`
  - 用 `curl http://127.0.0.1:18088/healthz` 测
- **`clawExitCode=1`**
  - 多数是模型凭证没配（如 `ANTHROPIC_API_KEY`）
- **MCP 没加载**
  - 先看 `/healthz` 里的 `defaultHttpMcpUrl`
  - 再看 `/v1/mcp/injected/{dsId}` 的 `mcpReport.servers`

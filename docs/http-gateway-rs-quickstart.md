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

### 1) 准备环境文件

```bash
cp deploy/podman/.env.example deploy/podman/.env
```

至少改这两个：

- `CLAUDE_TAP_IMAGE`
- `CLAW_DEFAULT_HTTP_MCP_URL`（容器内地址，通常 `http://claude-tap:<port>/mcp`）

### 2) 构建网关镜像

```bash
./deploy/podman/build.sh
```

### 3) 启动

```bash
./deploy/podman/up.sh
```

### 4) 联通性检查

```bash
./deploy/podman/check-connectivity.sh
```

### 5) 停止

```bash
./deploy/podman/down.sh
```

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

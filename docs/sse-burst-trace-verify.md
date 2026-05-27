# SSE burst 定位验证（`release-v1.4.2.sse` 类 tag）

Author: kejiqing

## 启用（不改业务行为）

在仓库根 `.env`（会挂进 worker / pool 运行时）增加：

```bash
CLAW_SSE_BURST_TRACE=1
CLAW_SSE_BURST_LOG_FILE=/var/lib/claw/workspace/sse-burst-trace.ndjson
```

宿主机 pool-daemon 若不在容器内，可写：

```bash
CLAW_SSE_BURST_LOG_FILE=/home/admin/work/claw-code/deploy/stack/claw-logs/sse-burst-trace.ndjson
```

## 部署 tag

```bash
./deploy/stack/gateway.sh up --release release-v1.4.2.sse
```

CI：push tag `release-v*`（含 `release-v1.4.2.sse`）触发 GHCR/ACR 构建（见 `.github/workflows/claw-code-image.yaml`）。

## 复现一次 solve 后分析

NDJSON 每行一种 `ev`：

| ev | 含义 |
|----|------|
| `http_chunk` | reqwest 一次 `chunk()` |
| `text_delta` | 该 chunk 内第 N 个文本 delta |
| `worker_emit` | `emit_report_delta` 写 stdout |
| `pool_ingest` | hub 收到 `report.delta` |

**判定：**

- 同一 `rawChunk` 对应多条 `text_delta` / `worker_emit` → 根因在 **HTTP 读包内多帧（A）**
- `worker_emit` 分散、`pool_ingest` 同 `readerBatchId` 成批 → **docker stdout / pool 读（B）**
- `pool_ingest` 分散、仅 gateway SSE 同 ms → **gateway 发出（C）**（与当前 ingest 日志不符时排除）

可与 tap `GET /api/sessions/traces?session=...` 对照同一 session。

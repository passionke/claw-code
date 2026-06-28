# SSE burst 定位验证（`release-v1.4.2.sse` 类 tag）

> **历史：** 下文针对宿主机 `claw-sandbox` pool。FC-only 栈请在 FC worker guest 内查 `CLAW_SSE_BURST_*` env。

Author: kejiqing

## 唯一配置面：仓库根 `.env`

**只维护仓库根 `.env`**。不要写两行 `CLAW_SSE_BURST_LOG_FILE`，不要写 `/claw_host_root/...`（那是容器内 session 挂载名，不是给 `.env` 用的）。

远程 Docker pool（如 252：`pool-daemon` 跑宿主机、worker 跑容器）在 `.env` 里**固定**下面三行：

```bash
CLAW_SSE_BURST_TRACE=1
CLAW_SSE_BURST_LOG_FILE=/home/admin/work/claw-code/deploy/stack/claw-logs/sse-burst-trace.ndjson
CLAW_POOL_WORKER_RUN_EXTRA="--add-host host.docker.internal:host-gateway -v /home/admin/work/claw-code/deploy/stack/claw-logs:/home/admin/work/claw-code/deploy/stack/claw-logs:rw"
```

说明：

- **一条日志路径**：宿主机 `deploy/stack/claw-logs/sse-burst-trace.ndjson`；pool 与 worker 写**同一文件**。
- **`CLAW_POOL_WORKER_RUN_EXTRA`**：整段用**双引号**包起来（`gateway.sh` 会 `source .env`，无引号时空格会把 `host.docker.internal:host-gateway` 当成命令执行）。
- 把 `claw-logs` 挂进 worker，否则 worker 在容器内打不开宿主机路径，`http_chunk` / `worker_emit` 会静默丢失。
- 若你已有 `CLAW_POOL_WORKER_RUN_EXTRA`，在同一行里**追加** `-v .../claw-logs:...:rw`，不要拆成第二个 `LOG_FILE`。

改完后（仓库根）：

```bash
mkdir -p deploy/stack/claw-logs
./deploy/stack/gateway.sh up
```

确保 **pool-daemon 已重启**并读到新 `.env`。**不必**为 burst trace 单独 `pack-deploy`（镜像需已含 `release-v1.4.2.sse` 打点代码）。

## 部署前自检（252 上复制执行）

```bash
grep CLAW_SSE_BURST /home/admin/work/claw-code/.env
tr '\0' '\n' < /proc/$(pgrep -f claw-sandbox | head -1)/environ | grep CLAW_SSE_BURST
docker exec $(docker ps -q -f name=claw-worker- | head -1) \
  touch /home/admin/work/claw-code/deploy/stack/claw-logs/.worker-ok && echo worker_write_OK
curl -fsS http://127.0.0.1:18088/healthz >/dev/null && echo gateway_OK
```

**必须**出现 `worker_write_OK`，否则不要开始复现。

## 复现后

```bash
wc -l /home/admin/work/claw-code/deploy/stack/claw-logs/sse-burst-trace.ndjson
grep -o '"ev":"[^"]*"' /home/admin/work/claw-code/deploy/stack/claw-logs/sse-burst-trace.ndjson | sort | uniq -c
```

应同时有 `http_chunk`、`worker_emit`、`pool_ingest`。

若只有 `pool_ingest`：确认 worker 镜像含 `WORKER_ENV_KEYS` 里的 `CLAW_SSE_BURST_*`（`gateway-solve-turn` 白名单注入；仅挂 `worker.env` 不够）。改 keys 后需重编 worker 镜像并 `gateway.sh up`。

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
- `pool_ingest` 分散、仅 gateway SSE 同 ms → **gateway 发出（C）**

可与 tap `GET /api/sessions/traces?session=...` 对照同一 session。分析 report 正文时跳过 81s 前无 `report.delta` 的阶段，专看正文段的 burst 节奏。

## 镜像 tag

```bash
./deploy/stack/gateway.sh up --release release-v1.4.2.sse_burst
```

CI：push tag `release-v*` 触发镜像构建（见 `.github/workflows/claw-code-image.yaml`）。

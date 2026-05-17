# Claw Web — 部署后功能验证清单

Author: kejiqing

**唯一入口命令**（仓库根目录）：

```bash
./tests/verify-claw-web.sh --tier all
```

| 层级 | 功能 | 契约 | smoke（无 gateway） | full（gateway 已 up） |
|------|------|------|---------------------|----------------------|
| L0 | 标识符 / 契约文件 | L0, README | 是 | 是 |
| L1 | bridge `healthz` | L1 | 是 | 是 |
| L1 | bridge `POST /v1/agent/run` SSE（RUN_* + TEXT_*） | L1 | mock 模式 | 真实模式 |
| L2 | gateway `GET /v1/events/{taskId}` NDJSON | L2 | cargo test | curl |
| L2 | `solve_async` 写入 `solve.queued` tap | L2 | cargo test | curl + dev seed 或真实 solve |
| L2 | bridge → gateway `solve_async` 链路 | L2 | **integration test** | curl agent/run |
| L3 | worker / 任务终态 | L3 | — | 任务 API 或 dev seed |
| L4 | `POST /v1/interrupts/{id}/resolve` | L4 | cargo test | dev seed + resolve |
| L4 | bridge 转发 `interrupt.required` | L4 | unit test | dev seed + agent/run（可选） |
| L5 | JWT `401` / tenant `403` | L5 | cargo test | 需 `CLAW_GATEWAY_AUTH=1` 重启 gateway |
| L5 | `GET /v1/audit` | L5 | cargo test | auth 开启后 curl |
| 部署 | compose 含 gateway + bridge | claw-web-stack | — | `gateway.sh verify-web` |

**full 前置**：`./deploy/stack/gateway.sh up` 且 `.env` 含 `CLAW_GATEWAY_DEV_AGUI=1`（dev seed 用）。

# AG-UI + A2UI 过程披露契约

Author: kejiqing

Agent↔UI **过程披露**的权威说明。BOSS 报告正文仍走 [`live-report-contract.md`](live-report-contract.md) 的 `biz.report.*`（**不改、不弃用**）。

---

## 1. 三层壳

| 层 | 协议 | 职责 |
|---|---|---|
| 第1层 | OpenAI Responses `POST /v1/responses` | Agent 入口/完成语义；`stream=true` 真流灌 LiveReportHub |
| 第2层 | **AG-UI** | 跑中事件流（生命周期、工具、状态、HITL） |
| 第3层 | **A2UI** | 可渲染内容（`claw-ask/v1`、`claw-process/v1`） |
| 并列 | `biz.report.*` | 报告正文老定制 SSE |

```text
LiveReportHub（唯一 SSE 源）
    ├─→ biz.report.*     （报告正文，语义不动）
    ├─→ AG-UI + A2UI     （过程 / UI 内容）
    └─→ Responses stream （stream=true 真流）
```

---

## 2. Worker stdout 事件（扩展）

前缀仍为 `__CLAW_GATEWAY_STDOUT__`。既有：`report.delta`、`ask.user`、`ask.user.cleared`、`solve.done`、`delegate.*`。

新增：

| `ev` | 字段 | 说明 |
|---|---|---|
| `tool.start` | `toolCallId`, `name`, `kind`, `title`, `argsSummary?` | 工具开始 |
| `tool.end` | `toolCallId`, `name`, `status` (`ok`/`error`), `durationMs`, `resultSummary?` | 工具结束 |
| `progress` | `kind`, `message`, `tsMs?` | 进度一行（可选；`report_progress` 仍可走 `report.delta`） |

`kind` 建议：`search` / `read` / `edit` / `shell` / `mcp` / `delegate` / `tool`。

---

## 3. AG-UI 出口

`GET /v1/ag-ui/runs?sessionId=&turnId=&projId=` → `text/event-stream`

每条 SSE `data` 为 JSON，`type` 字段对齐 AG-UI 常见命名：

| type | 何时 |
|---|---|
| `RUN_STARTED` | 订阅开始 |
| `TOOL_CALL_START` / `TOOL_CALL_END` / `TOOL_CALL_RESULT` | `tool.*` |
| `STATE_DELTA` | `progress` |
| `CUSTOM` `name=a2ui` | A2UI surface（ask / process） |
| `CUSTOM` `name=nerogate.delegate` | delegate 控制（若 ingest） |
| `RUN_FINISHED` / `RUN_ERROR` | `solve.done` / 失败 |

AskUser：`ask.user` → `CUSTOM` `{ "name":"a2ui", "value": <claw-ask/v1> }`（进入 AG-UI 流；`biz.report` SSE 仍不推 AskUser）。

---

## 4. A2UI catalogs

### `claw-ask/v1`（既有）

HITL：`Text` / `MultipleChoice` / `TextField` / `Button`。

### `claw-process/v1`（新建）

过程步骤列表。Gateway projector 从 `tool.*` 合成，worker 不必拼 A2UI。

```json
{
  "version": "0.8",
  "catalogId": "claw-process/v1",
  "surfaceId": "process-<turnId>",
  "components": [
    {
      "id": "step-<toolCallId>",
      "component": "ProcessStep",
      "kind": "search",
      "title": "Searching \"订单\"",
      "status": "ok",
      "durationMs": 800,
      "summary": "12 hits"
    }
  ]
}
```

产品形态：L1 一行活动；L2/L3 由客户端按 `kind` 展开（见实现计划「过程披露长什么样」）。

---

## 5. Responses `stream=true`

1. 先开 SSE，再 `solve_async` 入队（持有 `turnId`）。
2. 订 LiveReportHub，投影：
   - `report.delta` → `response.output_text.delta`
   - `tool.*` → function_call 相关中间事件（有则发）
   - 终态 → `response.completed` + `done`/`[DONE]`
3. **禁止**先 `await` 整轮再假 SSE。

非 stream 仍可同步整包返回。

---

## 6. 与 live_report 关系

- `biz.report.*` **协议不变**。
- Hub **扩展 ingest**，报告分支逻辑不动。
- Admin：报告继续 `useBizReportStream`；过程订 AG-UI。

---

## 7. 变更记录

| 日期 | 说明 |
|------|------|
| 2026-09-07 | 初版：AG-UI 传输 + A2UI 内容；Hub 单源；biz.report 并列不动 |

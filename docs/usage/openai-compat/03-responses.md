# Responses API 用法

Author: kejiqing

路径：`POST /v1/responses`  
鉴权：`Authorization: Bearer ngmk_…`

OpenAI Responses 形状的入口，与 Chat Completions 共用同一 Agent solve kernel。适合已经按 Responses 协议集成的客户端；多数集成方用 Chat Completions 即可。

## 请求体

| 字段 | 必填 | 说明 |
|------|------|------|
| `model` | 是 | 同 Chat：等于 Key 的 `modelAlias` 或 `proj-{projId}` |
| `input` | 是 | 字符串，或含文本的数组（见下） |
| `instructions` | 否 | 补充说明，会拼进 prompt 前部 |
| `conversation` | 否 | **稳定会话键**（对应 Chat 的 `user`） |
| `previous_response_id` | 否 | 上一轮响应的 `id`（即上一轮 `turnId`）；用于续同一 session |
| `stream` | 否 | 默认 `false`；语义同 Chat（先跑完再 SSE） |
| `timeout` | 否 | 秒 |
| `extra_session` | 否 | JSON 对象，同 solve `extraSession` |
| `tools` | 否 | 非空 → `400 unsupported_feature` |

### `input` 形态

- 字符串：非空即可  
- 数组：拼接每项字符串，或项上的 `content` / `text` 文本；拼完后仍须非空  

### 示例：同步

```bash
curl -sS -X POST "$GATEWAY/v1/responses" \
  -H "Authorization: Bearer $NGMK_TOKEN" \
  -H 'Content-Type: application/json' \
  -d '{
    "model": "agent",
    "conversation": "demo-conversation-001",
    "instructions": "回答尽量简短",
    "input": "今天有什么经营建议？"
  }'
```

## 响应（非 stream）

```json
{
  "id": "T_…",
  "object": "response",
  "created_at": 1710000000,
  "status": "completed",
  "model": "agent",
  "output": [{
    "type": "message",
    "id": "T_…_msg",
    "role": "assistant",
    "content": [{"type": "output_text", "text": "……"}]
  }],
  "usage": {
    "input_tokens": 123,
    "output_tokens": 45,
    "total_tokens": 168,
    "input_tokens_details": { "cached_tokens": 10 }
  },
  "nerogate": {
    "sessionId": "……",
    "turnId": "T_…",
    "usageByModel": [
      {
        "model": "…",
        "input_tokens": 123,
        "output_tokens": 45,
        "total_tokens": 168,
        "input_tokens_details": { "cached_tokens": 10 }
      }
    ]
  }
}
```

| 字段 / 头 | 含义 |
|-----------|------|
| `id` | 本轮 `turnId`；下一轮可作 `previous_response_id` |
| `x-nerogate-session-id` | `sessionId` |
| `status` | 成功完成时为 `completed` |
| `usage` | 本轮 `gateway_model_usage` 合计（Responses 字段名）；无记账行时为 `null` |
| `nerogate.usageByModel` | 按模型拆分 |

## 续聊：两种方式

### 1. `conversation`（推荐与 Chat 的 `user` 对齐）

同一 Key 下相同 `conversation` → 复用同一 `sessionId`（会话仍存在时）。  
网关会把映射写入 `gateway_openai_conversation`。

### 2. `previous_response_id`

传入上一轮响应的 `id`：

1. 网关查 `gateway_openai_response` 得到当时的 `sessionId`
2. 校验该记录属于**当前 Key / 当前项目**
3. 本轮在同一 session 上继续

找不到或归属不符 → `400` / `403`。

**优先级**：实现上若同时提供 `previous_response_id`，以它解析出的 session 为准（见路由 `resolve_session`）。

## 流式（`stream: true`）

同样是：先同步跑完 Agent，再发 SSE：

1. 事件名 `response.completed`，data 为完整 response JSON  
2. 事件名 `done`，data 为 `[DONE]`

不要期望 token 级增量。

## 与 Chat Completions 怎么选

| 你的客户端 | 建议 |
|------------|------|
| OpenAI Python/Node `chat.completions` | 用 `/v1/chat/completions` |
| 已接 Responses / `responses.create` | 用 `/v1/responses` |
| 需要 `previous_response_id` 链式续聊 | Responses 更直接；Chat 侧用稳定 `user` 键即可 |

两边能力等价于「一轮 Agent 完成」；差别主要在请求/响应 JSON 形状与续聊字段名。

## 相关文档

- 选型：[00-choose-api.md](00-choose-api.md)
- Chat Completions：[02-chat-completions.md](02-chat-completions.md)
- SDK 与限制：[04-sdk-and-limits.md](04-sdk-and-limits.md)

## Mind 副本

- 本文：https://mind.maxiot-inc.com/docs/ce86919c-dd25-409d-8185-8f1521bfe16e
- 系列文件夹：[OpenAI 兼容接口](https://mind.maxiot-inc.com/folders/cd10ae3f-e32d-4ed0-991b-ca636eb0e405)（父级 [NeruoGate](https://mind.maxiot-inc.com/folders/3f2db9e2-d9d3-4135-b705-402ec3f9521f)）

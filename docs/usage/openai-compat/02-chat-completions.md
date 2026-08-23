# Chat Completions 用法

Author: kejiqing

路径：`POST /v1/chat/completions`  
鉴权：`Authorization: Bearer ngmk_…`

本文说明请求字段、续聊、流式与常见错误。实现与 [`agent_completion.rs`](../../../rust/crates/http-gateway-rs/src/agent_completion.rs) / [`openai_compat.rs`](../../../rust/crates/http-gateway-rs/src/routes/openai_compat.rs) 对齐。

## 请求体

| 字段 | 必填 | 说明 |
|------|------|------|
| `model` | 是 | 须等于 Key 的 `modelAlias`（常见 `agent`），或 `proj-{projId}` |
| `messages` | 是 | 非空数组；至少一条非空 `user` |
| `user` | 否 | **稳定会话键**；有则映射/复用 gateway `sessionId` |
| `stream` | 否 | 默认 `false`；见下文「流式」 |
| `timeout` | 否 | 秒；传给底层 solve 的 `timeoutSeconds` |
| `extra_session` | 否 | JSON 对象；等价于 solve 的 `extraSession`（业务上下文） |
| `tools` | 否 | **必须为空或不传**；非空 → `400 unsupported_feature` |

### messages 如何变成 Agent prompt

网关**不会**把整段 messages 原样塞给上游 LLM 协议，而是归一成**单轮** `userPrompt`：

1. `system` / `developer` → 拼进「补充说明」
2. 更早的 `user` / `assistant` → 拼进「Prior conversation」摘要
3. **最后一条非空 `user`** → 「Current user request」

因此：多轮上下文应以 **`user` 会话键 + 网关侧 session 历史** 为主；仅靠客户端反复塞超长 messages 并不是推荐模式。

`content` 可为字符串，或 OpenAI 常见的数组（取其中 `text` 段拼接）。

### 示例：同步

```bash
curl -sS -X POST "$GATEWAY/v1/chat/completions" \
  -H "Authorization: Bearer $NGMK_TOKEN" \
  -H 'Content-Type: application/json' \
  -d '{
    "model": "agent",
    "user": "ops-user-42",
    "timeout": 300,
    "extra_session": {
      "tenant_code": "demo",
      "store_id": "S001"
    },
    "messages": [
      {"role": "system", "content": "回答尽量简短"},
      {"role": "user", "content": "今天有什么经营建议？"}
    ]
  }'
```

若项目配置了 `extra_session_fields_json`，`extra_session` 必须满足项目字段校验（与 `/v1/solve` 相同规则）。

## 响应（非 stream）

标准 Chat Completions 形状，并附加 Nerogate 扩展：

```json
{
  "id": "T_…",
  "object": "chat.completion",
  "created": 1710000000,
  "model": "agent",
  "choices": [{
    "index": 0,
    "message": {"role": "assistant", "content": "……"},
    "finish_reason": "stop"
  }],
  "usage": {
    "prompt_tokens": 123,
    "completion_tokens": 45,
    "total_tokens": 168,
    "prompt_tokens_details": { "cached_tokens": 10 }
  },
  "nerogate": {
    "sessionId": "……",
    "turnId": "T_…",
    "usageByModel": [
      {
        "model": "…",
        "prompt_tokens": 123,
        "completion_tokens": 45,
        "total_tokens": 168,
        "prompt_tokens_details": { "cached_tokens": 10 }
      }
    ]
  }
}
```

| 字段 / 头 | 含义 |
|-----------|------|
| `id` / `nerogate.turnId` | 本轮 `turnId` |
| `x-nerogate-session-id` | 本轮 `sessionId` |
| `usage` | 本轮在 observe tap 落库的 `gateway_model_usage` **按 turn 合计**；无记账行时为 `null`（不编造） |
| `nerogate.usageByModel` | 按模型拆分的同口径合计（OpenAI 标准 `usage` 无按模型字段） |

口径：`prompt_tokens = input + cache_creation + cache_read`，`completion_tokens = output`，`prompt_tokens_details.cached_tokens = cache_read`。真源是 tap（须带 `claw-turn-id`）；Gateway 只做 SUM，不扫 Live / `tap-traces/`。

正文来自 solve 结果中的报告/消息文本（与同步 solve 终态一致的提取逻辑）。

## 续聊

推荐固定传 `user`（业务侧用户 id、会话 id、租户内对话 id 等，**同一对话保持不变**）：

1. 首次：无映射 → 网关新建 session，并记下 `(api_key_id, user) → sessionId`
2. 后续：同一 Key + 同一 `user` → 复用该 `sessionId`（会话目录仍存在时）

不传 `user`：每轮通常新建会话（除非你另有映射手段——Chat Completions 路径不读 Responses 的 `previous_response_id`）。

## 流式（`stream: true`）

**重要：这不是 token 级打字机。**

实际行为：

1. 网关先**同步**跑完整轮 Agent（与非 stream 相同内核）
2. 再通过 SSE 推送：一条带完整 `content` 的 `chat.completion.chunk`，再一条 `finish_reason=stop`，最后 `data: [DONE]`

因此：客户端仍会阻塞到 Agent 结束；`stream=true` 主要兼容「只吃 SSE」的 SDK，**不能**用来提前看到半截推理过程。若要过程进度，请用 `/v1/solve_async` + 任务/报告接口。

响应头同样带 `x-nerogate-session-id`；并设置 `x-accel-buffering: no` 以减少反向代理缓冲。

## 错误形状

错误体为 OpenAI 风格：

```json
{
  "error": {
    "message": "……",
    "type": "invalid_request_error",
    "code": "invalid_api_key",
    "param": null
  }
}
```

| HTTP | code（典型） | 场景 |
|------|--------------|------|
| 401 | `invalid_api_key` | 缺 Bearer / Key 无效 |
| 400 | `invalid_request` | messages 空、无 user 内容、`previous` 类无效等 |
| 400 | `model_not_found` | `model` 与 Key 绑定不符 |
| 400 | `unsupported_feature` | 非空 `tools` |
| 403 | `permission_denied` | 会话键/响应属于其他项目或 Key |
| 4xx/5xx | `server_error` 等 | 底层 solve 失败时包装 |

## 不要做的事

- 在 body 里传 `projId` 指望换项目 → **无效**；项目由 Key 决定
- 传 OpenAI function `tools` 指望 Agent 回调你的函数 → **拒绝**
- 假设 `stream=true` 会边生成边推 token → **不会**
- 用本接口上传附件 / 指定 `allowedTools` → 请改走 `/v1/solve` 或 `/v1/solve_async`

## 相关文档

- 选型：[00-choose-api.md](00-choose-api.md)
- 签发 Key：[01-quickstart.md](01-quickstart.md)
- Responses：[03-responses.md](03-responses.md)
- SDK 与限制：[04-sdk-and-limits.md](04-sdk-and-limits.md)

## Mind 副本

- 本文：https://mind.maxiot-inc.com/docs/388efe1e-68cd-4486-b97c-1ae2a5eed975
- 系列文件夹：[OpenAI 兼容接口](https://mind.maxiot-inc.com/folders/cd10ae3f-e32d-4ed0-991b-ca636eb0e405)（父级 [NeruoGate](https://mind.maxiot-inc.com/folders/3f2db9e2-d9d3-4135-b705-402ec3f9521f)）

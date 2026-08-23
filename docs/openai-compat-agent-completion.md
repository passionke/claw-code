# OpenAI 兼容 Agent 完成接口（Usage 索引）

Author: kejiqing

Gateway 在历史 **`/v1/solve`**（常称 resolve）与 **`/v1/solve_async`**（常称 resolve_async）之外，提供标准 **OpenAI 兼容** 入口，便于 SQLBot、LangChain、OpenAI SDK 等直接对接，而无需自写 solve 适配器。

这是 **Agent 完成协议**（项目能力在服务端执行），**不是**把 Gateway 当作上游 LLM 代理。

## Usage 系列（推荐从这里读）

| 文档 | 内容 | Mind |
|------|------|------|
| [00 怎么选接口](usage/openai-compat/00-choose-api.md) | solve / solve_async / OpenAI 兼容选型 | [打开](https://mind.maxiot-inc.com/docs/dcf938fa-8c46-4d29-9a76-ca8681735173) |
| [01 快速开始](usage/openai-compat/01-quickstart.md) | 签发 `ngmk_` Key + 第一条请求 | [打开](https://mind.maxiot-inc.com/docs/799df734-1f87-4229-9f3f-bc5533f4168c) |
| [02 Chat Completions](usage/openai-compat/02-chat-completions.md) | messages、续聊、stream、错误 | [打开](https://mind.maxiot-inc.com/docs/388efe1e-68cd-4486-b97c-1ae2a5eed975) |
| [03 Responses](usage/openai-compat/03-responses.md) | `input` / `conversation` / `previous_response_id` | [打开](https://mind.maxiot-inc.com/docs/ce86919c-dd25-409d-8185-8f1521bfe16e) |
| [04 SDK 与限制](usage/openai-compat/04-sdk-and-limits.md) | Python SDK、硬性限制、与 solve 字段对照 | [打开](https://mind.maxiot-inc.com/docs/544e69c6-7fb7-4302-9993-7fa8f8d05479) |

## 端点速查

| Method | Path | Auth | Notes |
| --- | --- | --- | --- |
| `POST` | `/v1/chat/completions` | Bearer `ngmk_…` | OpenAI Chat Completions 形状 |
| `POST` | `/v1/responses` | Bearer `ngmk_…` | OpenAI Responses 形状 |
| `GET` / `POST` | `/v1/projects/{projId}/model-api-keys` | 可选 `camt_…` | 列表 / 签发 |
| `DELETE` | `/v1/projects/{projId}/model-api-keys/{id}` | 可选 `camt_…` | 吊销 |

## 契约要点

1. Bearer Key → 绑定 `projId` + `modelAlias`（客户端不能改项目）。
2. messages / input → 归一成单轮 `userPrompt`，进入既有 solve kernel。
3. Chat 的 `user` 或 Responses 的 `conversation` → 稳定会话键 → gateway `sessionId`。
4. 响应 `id` = gateway `turnId`；头 `x-nerogate-session-id` = session。
5. 非空 OpenAI `tools` → `400 unsupported_feature`（Agent 工具只在网关侧）。
6. `stream=true`：先同步跑完，再发最终 content（或 `response.completed`）与 `[DONE]`，**不是** token 流。
7. `usage`：本轮在 claw-tap 落库的 token 合计（`gateway_model_usage` SUM）；无行则 `null`。按模型见 `nerogate.usageByModel`。

## 签发 Key（一行）

```bash
curl -sS -X POST "$GATEWAY/v1/projects/1/model-api-keys" \
  -H 'Content-Type: application/json' \
  -d '{"name":"sqlbot","modelAlias":"agent"}'
```

请妥善保存返回的 `token`；库中只持久化 hash。

## 相关

- 接口详表：[`http-gateway-rs-api.md`](http-gateway-rs-api.md)（含 OpenAI Compat 小节）
- 实现：`rust/crates/http-gateway-rs/src/routes/openai_compat.rs`、`agent_completion.rs`

## Mind 副本

- 本文：https://mind.maxiot-inc.com/docs/7955ce62-269a-4450-b731-71f0af2e6d9c
- 系列文件夹：[OpenAI 兼容接口](https://mind.maxiot-inc.com/folders/cd10ae3f-e32d-4ed0-991b-ca636eb0e405)（父级 [NeruoGate](https://mind.maxiot-inc.com/folders/3f2db9e2-d9d3-4135-b705-402ec3f9521f)）

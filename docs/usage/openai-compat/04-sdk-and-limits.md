# SDK 示例、限制与字段对照

Author: kejiqing

面向已经选型 OpenAI 兼容入口的集成方：如何用官方 SDK 指向 Gateway，以及**明确不能假设的行为**。

## Base URL 与 Key

| 配置项 | 值 |
|--------|-----|
| Base URL | `$GATEWAY/v1`（注意多数 SDK 会自己拼 `/chat/completions`） |
| API Key | 签发得到的 `ngmk_…` |
| model | Key 的 `modelAlias`（默认 `agent`）或 `proj-{id}` |

**不要**把 `OPENAI_API_KEY` 指向真实 OpenAI，却把 Base URL 指到 Gateway（或反过来）混用；两边语义不同。

## Python：OpenAI SDK（Chat Completions）

```bash
pip install openai
```

```python
# -*- coding: utf-8 -*-
# Author: kejiqing
from openai import OpenAI

client = OpenAI(
    api_key="ngmk_…",
    base_url="http://127.0.0.1:18088/v1",
)

resp = client.chat.completions.create(
    model="agent",
    user="demo-conversation-001",
    messages=[
        {"role": "user", "content": "用一句话介绍你能做什么"},
    ],
)

print(resp.choices[0].message.content)
print(resp.id)  # turnId
# 部分版本可从 raw response headers 读 x-nerogate-session-id
```

流式（仍会等 Agent 整轮结束后再收到大块 content）：

```python
stream = client.chat.completions.create(
    model="agent",
    user="demo-conversation-001",
    messages=[{"role": "user", "content": "简要总结今日重点"}],
    stream=True,
)
for chunk in stream:
    delta = chunk.choices[0].delta.content
    if delta:
        print(delta, end="", flush=True)
```

若 SDK 版本支持 Responses：

```python
r = client.responses.create(
    model="agent",
    conversation="demo-conversation-001",
    input="用一句话介绍你能做什么",
)
# 下一轮可用 previous_response_id=r.id
```

`extra_session` 若 SDK 无一等字段，可用底层 HTTP / `extra_body`（视 SDK 版本而定）传入；或改用 curl。

## curl 最小模板

```bash
# Chat
curl -sS -X POST "$GATEWAY/v1/chat/completions" \
  -H "Authorization: Bearer $NGMK_TOKEN" \
  -H 'Content-Type: application/json' \
  -d '{"model":"agent","messages":[{"role":"user","content":"ping"}]}'

# Responses
curl -sS -X POST "$GATEWAY/v1/responses" \
  -H "Authorization: Bearer $NGMK_TOKEN" \
  -H 'Content-Type: application/json' \
  -d '{"model":"agent","input":"ping"}'
```

## 硬性限制（请勿踩坑）

1. **不是模型代理**：`model` 不是任意上游模型 id；项目推理模型由网关/项目配置决定。  
2. **禁止客户端 `tools`**：非空 → `unsupported_feature`。工具在 Agent / 项目侧执行。  
3. **无异步 task 轮询**：没有 `taskId`；要进度请用 `/v1/solve_async`。  
4. **stream ≠ token 流**：先整轮 solve，再推最终内容（或 `response.completed`）。  
5. **无本路径 attachments / allowedTools**：需要附件或收紧工具白名单时走原生 solve。  
6. **Key 绑定项目**：换项目 = 换 Key，不能靠 body 改 `projId`。  
7. **明文 token 只出现一次**：创建响应里的 `token` 须自行保管；列表接口只返回前缀与元数据。  
8. **`usage` 按 turn 合计**：来自 observe tap 写入的 `gateway_model_usage`（须出站带 `claw-turn-id`）。表空则为 `null`，不要把 worker 内部 `TokenUsage` 或 Live HTML 当账单。多模型看 `nerogate.usageByModel`。

## 与原生 solve 字段对照

| 概念 | `/v1/solve` | OpenAI 兼容 |
|------|-------------|-------------|
| 项目 | body `projId` | Key → `projId` |
| 用户输入 | `userPrompt` | messages / `input`（归一化） |
| 会话 | `sessionId` 或头 | Chat：`user`；Responses：`conversation` / `previous_response_id` |
| 业务上下文 | `extraSession` | `extra_session` |
| 超时 | `timeoutSeconds` | `timeout` |
| 工具白名单 | `allowedTools` | 不暴露 |
| 附件 | `attachments` | 不暴露 |
| 轮次 id | 响应 `turnId` | 响应 `id` / `nerogate.turnId` |
| 会话 id | 响应 `sessionId` | 头 `x-nerogate-session-id` / `nerogate.sessionId` |

## 管理端点速查

| 方法 | 路径 | 用途 |
|------|------|------|
| `POST` | `/v1/projects/{projId}/model-api-keys` | 签发（返回明文 `token` 一次） |
| `GET` | `/v1/projects/{projId}/model-api-keys` | 列表 |
| `DELETE` | `/v1/projects/{projId}/model-api-keys/{id}` | 吊销 |

## LangChain / 其他

凡支持「自定义 OpenAI-compatible base URL」的适配器，一般把：

- `openai_api_base` / `base_url` → `$GATEWAY/v1`
- `api_key` → `ngmk_…`
- `model` → `agent`（或你的 alias）

并关闭客户端 tool-calling，或确保不向接口提交 `tools`。

若框架强制异步 poll 或假想 token 流，请评估改用 `/v1/solve_async`，而不是硬套本接口。

## 系列索引

- [00 选型](00-choose-api.md)
- [01 快速开始](01-quickstart.md)
- [02 Chat Completions](02-chat-completions.md)
- [03 Responses](03-responses.md)
- 总索引：[`../../openai-compat-agent-completion.md`](../../openai-compat-agent-completion.md)

## Mind 副本

- 本文：https://mind.maxiot-inc.com/docs/544e69c6-7fb7-4302-9993-7fa8f8d05479
- 系列文件夹：[OpenAI 兼容接口](https://mind.maxiot-inc.com/folders/cd10ae3f-e32d-4ed0-991b-ca636eb0e405)（父级 [NeruoGate](https://mind.maxiot-inc.com/folders/3f2db9e2-d9d3-4135-b705-402ec3f9521f)）

# 怎么选接口：solve / solve_async / OpenAI 兼容

Author: kejiqing

Gateway 对外有三条常用入口。外部有时把前两条叫 **resolve** / **resolve_async**；代码路径分别是 `/v1/solve` 与 `/v1/solve_async`。第三条是标准 **OpenAI 兼容** 形态，方便 OpenAI SDK、LangChain、SQLBot 等直接对接，而**不必**自写 solve 适配器。

本文只帮你选型；具体调用见同系列后续文档。

## 三条入口一览

| 入口 | 路径 | 鉴权（典型） | 阻塞方式 | 适合谁 |
|------|------|--------------|----------|--------|
| 同步 solve | `POST /v1/solve` | 内网 / Admin 等既有约定 | HTTP 一直等到本轮结束 | 自建 BFF、已熟悉 `projId` + `userPrompt` 的调用方 |
| 异步 solve | `POST /v1/solve_async` + `GET /v1/tasks/{taskId}` | 同上 | 立即返回 `taskId`，再轮询 | 长任务、要进度/取消、要 live 报告的产品侧 |
| OpenAI 兼容 | `POST /v1/chat/completions` 或 `POST /v1/responses` | Bearer `ngmk_…` 项目模型 Key | **同步**跑完 Agent 再返回（`stream=true` 也是先跑完再推一条 SSE） | 已有 OpenAI 客户端、不想学 solve 契约的集成方 |

三条路底层都进**同一套 Agent solve kernel**（项目配置、技能、MCP、沙箱工具）。差别主要在：**请求形状、鉴权、会话续聊字段、是否支持异步轮询**。

## 先问自己三个问题

1. **调用方能不能改请求体？**  
   - 只能配 `base_url` + `api_key` + `model`（OpenAI SDK / 部分 SaaS）→ 选 **OpenAI 兼容**。  
   - 可以发自定义 JSON（`projId`、`allowedTools`、`attachments`）→ 可继续用 **solve / solve_async**。

2. **要不要异步与进度？**  
   - 需要 `queued` / `running`、取消、`biz_advice_report` live SSE → 用 **solve_async**。  
   - OpenAI 兼容路径**没有** `taskId` 轮询；一次 HTTP 对应一轮完整 Agent 执行。

3. **要不要客户端传 OpenAI `tools`？**  
   - Agent 侧工具（bash、MCP、Skill 等）由项目配置与网关策略决定，**不在** OpenAI 请求里声明。  
   - 若请求带非空 `tools`，OpenAI 兼容接口返回 `400`（`unsupported_feature`）。需要客户端 tool-calling 协议时，不要用这条入口。

## 推荐选型

```text
已有 OpenAI / LangChain / SQLBot 模型客户端
        │
        ├─ 只要「问一句 → Agent 答一句」、可接受同步等待
        │         → /v1/chat/completions 或 /v1/responses
        │
        └─ 要轮询进度、取消、live 报告
                  → 仍用 /v1/solve_async（不要硬套 OpenAI 兼容）

自建 BFF，已熟悉 projId / sessionId / attachments
        │
        ├─ 短请求、可阻塞          → /v1/solve
        └─ 长任务 / 进度 / 取消    → /v1/solve_async
```

## 重要认知：不是「OpenAI 模型代理」

OpenAI 兼容接口把 Chat Completions / Responses 的请求**归一化成一次 Agent solve**：

- Bearer Key 绑定 **`projId` + `modelAlias`**，客户端**不能**在 body 里改项目。
- `model` 字段表示 Key 上绑定的别名（常见为 `agent`），**不是**随便填一个上游模型名去「直连 GPT」。
- 响应里的 `id` 是网关 **`turnId`**；会话见响应头 `x-nerogate-session-id` 与 body 扩展字段 `nerogate`。

因此：把它当成「带项目能力的 Agent 完成协议」，而不是「把 Gateway 当 api.openai.com」。

## 字段能力对照（选型用）

| 能力 | solve / solve_async | OpenAI 兼容 |
|------|---------------------|-------------|
| 指定 `projId` | 请求体必填 | 由 Key 决定，body 不可改 |
| `userPrompt` / messages | `userPrompt` | messages / `input` 归一成一轮 prompt |
| 显式 `sessionId` | 支持 | 用 `user` / `conversation` / `previous_response_id` 映射 |
| `extraSession` | 支持 | 支持（字段名 `extra_session`） |
| `allowedTools` | 支持 | **本路径不暴露**（走项目/网关默认） |
| `attachments` | 支持 | **本路径不暴露** |
| 异步 `taskId` + 轮询 | solve_async | 无 |
| 标准 OpenAI SDK | 需适配器 | 开箱可用 |

## 下一步

- 第一次对接 OpenAI 兼容：[01-quickstart.md](01-quickstart.md)
- Chat Completions 细节：[02-chat-completions.md](02-chat-completions.md)
- Responses API：[03-responses.md](03-responses.md)
- SDK、限制与字段对照表：[04-sdk-and-limits.md](04-sdk-and-limits.md)

历史 solve 路径的接口详表见 [`../../http-gateway-rs-api.md`](../../http-gateway-rs-api.md)。

## Mind 副本

- 本文：https://mind.maxiot-inc.com/docs/dcf938fa-8c46-4d29-9a76-ca8681735173
- 系列文件夹：[OpenAI 兼容接口](https://mind.maxiot-inc.com/folders/cd10ae3f-e32d-4ed0-991b-ca636eb0e405)（父级 [NeruoGate](https://mind.maxiot-inc.com/folders/3f2db9e2-d9d3-4135-b705-402ec3f9521f)）

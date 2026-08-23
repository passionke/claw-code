# 快速开始：签发 Key + 第一条 Chat Completions

Author: kejiqing

目标：在 5 分钟内用 curl 打通 **OpenAI 兼容** 的第一条 Agent 请求。

前置：已有可用的 Gateway Base URL（下文用 `$GATEWAY`，例如 `https://gateway.example.com` 或 `http://127.0.0.1:18088`），且目标 **`projId` 已有 `project_config`**（Admin / `POST /v1/projects` 建过项目）。

## 1. 签发项目模型 API Key

OpenAI 兼容接口只认 Bearer **`ngmk_…`**（project model API key）。Key 绑定固定的 `projId` 与 `modelAlias`。

```bash
export GATEWAY='http://127.0.0.1:18088'
export PROJ_ID=1

curl -sS -X POST "$GATEWAY/v1/projects/$PROJ_ID/model-api-keys" \
  -H 'Content-Type: application/json' \
  -d '{"name":"external-demo","modelAlias":"agent","note":"usage quickstart"}'
```

成功时响应大致包含：

- `token`：明文 Key，**只在创建时返回一次**；请安全保存。库中只存 hash。
- `entry.projId` / `entry.modelAlias` / `entry.id`：元数据；列表与吊销用 `id`。

可选：若环境要求 Admin 令牌，可加：

```bash
-H "Authorization: Bearer camt_…"
```

未提供 `camt_` 时，行为与网关既有「可信内网可调管理类接口」约定一致（与签发路径上的 `require_admin_or_open` 一致）。

把明文写入环境变量（勿提交到仓库）：

```bash
export NGMK_TOKEN='ngmk_…'   # 粘贴上一步返回的 token
```

## 2. 列表 / 吊销（可选）

```bash
# 列表（不含明文 token）
curl -sS "$GATEWAY/v1/projects/$PROJ_ID/model-api-keys"

# 吊销（用 entry.id，例如 pmk-xxxxxxxx）
curl -sS -X DELETE "$GATEWAY/v1/projects/$PROJ_ID/model-api-keys/<key_id>"
```

## 3. 第一条 Chat Completions

`model` 必须与 Key 的 `modelAlias` 一致（默认签发时为 `agent`）。也可使用 `proj-{projId}`（例如 `proj-1`）。

```bash
curl -sS -X POST "$GATEWAY/v1/chat/completions" \
  -H "Authorization: Bearer $NGMK_TOKEN" \
  -H 'Content-Type: application/json' \
  -d '{
    "model": "agent",
    "messages": [
      {"role": "user", "content": "用一句话介绍你能做什么"}
    ]
  }'
```

关注这些返回：

| 位置 | 含义 |
|------|------|
| `choices[0].message.content` | Agent 本轮最终文本 |
| `id` | 网关 `turnId`（形如 `T_<hex>`） |
| 响应头 `x-nerogate-session-id` | 网关 `sessionId` |
| `nerogate.sessionId` / `nerogate.turnId` | 与上两者对应的扩展字段 |

这不是「调某个 GPT 模型名」，而是：**用该项目配置跑一轮 Agent**，再把结果包成 Chat Completions 形状。

## 4. 最小续聊（可选）

同一对话请固定传 OpenAI 字段 `user`（稳定会话键）。网关会把它映射到同一 `sessionId`：

```bash
curl -sS -X POST "$GATEWAY/v1/chat/completions" \
  -H "Authorization: Bearer $NGMK_TOKEN" \
  -H 'Content-Type: application/json' \
  -d '{
    "model": "agent",
    "user": "demo-conversation-001",
    "messages": [
      {"role": "user", "content": "记住：我的门店编码是 S001"}
    ]
  }'

curl -sS -X POST "$GATEWAY/v1/chat/completions" \
  -H "Authorization: Bearer $NGMK_TOKEN" \
  -H 'Content-Type: application/json' \
  -d '{
    "model": "agent",
    "user": "demo-conversation-001",
    "messages": [
      {"role": "user", "content": "我刚才说的门店编码是什么？"}
    ]
  }'
```

两次响应头里的 `x-nerogate-session-id` 应相同（在会话仍存在的前提下）。

## 5. 常见失败

| 现象 | 常见原因 |
|------|----------|
| `401` / `invalid_api_key` | 未带 Bearer、token 错误或已吊销 |
| `400` / `model_not_found` | `model` 与 Key 的 `modelAlias` 不一致 |
| `404` 签发 Key 时 | 该 `projId` 尚无 `project_config` |
| `400` / `unsupported_feature` | 请求里带了非空 `tools` |

## 下一步

- Chat Completions 全字段与 stream：[02-chat-completions.md](02-chat-completions.md)
- Responses API：[03-responses.md](03-responses.md)
- Python SDK：[04-sdk-and-limits.md](04-sdk-and-limits.md)

## Mind 副本

- 本文：https://mind.maxiot-inc.com/docs/799df734-1f87-4229-9f3f-bc5533f4168c
- 系列文件夹：[OpenAI 兼容接口](https://mind.maxiot-inc.com/folders/cd10ae3f-e32d-4ed0-991b-ca636eb0e405)（父级 [NeruoGate](https://mind.maxiot-inc.com/folders/3f2db9e2-d9d3-4135-b705-402ec3f9521f)）

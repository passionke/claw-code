# Preflight SPI v1（子进程 JSON 契约）

Author: kejiqing  
Schema: [`schemas/preflight-spi-v1.json`](../schemas/preflight-spi-v1.json)  
Related: [`ovs-chat/PREFLIGHT-SPI-PLAN.md`](ovs-chat/PREFLIGHT-SPI-PLAN.md)、[`gateway-solve-preflight.md`](gateway-solve-preflight.md)

## 概述

Gateway `PreflightRunner` 按项目配置的 `steps[]` 顺序执行 preflight。`impl.type=subprocess` 时，框架将 **JSON 请求写入子进程 stdin**，从 **stdout 读取 JSON 响应**。

- **spiVersion**：固定 `"1"`
- **超时**：`CLAW_PREFLIGHT_SUBPROCESS_TIMEOUT_SECS`（默认 120）
- **stdout 上限**：`CLAW_PREFLIGHT_SUBPROCESS_MAX_OUTPUT_BYTES`（默认 1 MiB）

## stdin：请求

```json
{
  "spiVersion": "1",
  "step": {
    "pluginId": "my_plugin",
    "scope": "every_turn",
    "config": {}
  },
  "context": {
    "sessionId": "sess-1",
    "turnId": "turn-1",
    "workDir": "/claw_host_root",
    "isContinuation": false,
    "userPrompt": "今天营业额多少",
    "priorUserPrompts": ["hello"],
    "extraSession": { "store_id": "1" },
    "model": "openai/deepseek-v4-pro"
  },
  "artifacts": [".claw/gateway-solve-session.jsonl"]
}
```

### scope

| 值 | 何时执行 |
|----|----------|
| `every_turn` | 每轮 solve |
| `session_first_turn` | 仅该 `sessionId` 首轮且步骤尚未 satisfied |

## stdout：响应

```json
{
  "status": "ok",
  "effects": [
    {
      "type": "lockLanguage",
      "language": "Chinese",
      "reason": "user message in Chinese"
    }
  ]
}
```

| status | 含义 |
|--------|------|
| `ok` | 应用 `effects` |
| `skip` | 无操作 |
| `error` | 记录 `message`，不应用 effects |

### effects（声明式）

| type | 字段 | subprocess 允许 |
|------|------|-----------------|
| `lockLanguage` | `language`, `reason?` | 是 |
| `writeSessionFile` | `relPath`, `content` | 是 |
| `appendSystemPromptSection` | `markdown` | 是 |
| `appendTranscriptSummary` | `text` | 是 |
| `injectToolExchange` | `toolName`, `input`, `output`, `isError?` | **否**（仅 builtin） |

子进程响应含 `injectToolExchange` 时，框架拒绝并记 error。

## builtin impl

```json
{ "type": "builtin", "handler": "turn_language" }
{ "type": "builtin", "handler": "sqlbot_mcp_start" }
```

内置 handler 在 `gateway-solve-turn` 内注册，不走子进程。

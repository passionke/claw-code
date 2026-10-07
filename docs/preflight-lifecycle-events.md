# Preflight 生命周期事件（项目扩展点）

Author: kejiqing

面向业务与插件作者：选事件 → 注册插件 → 配 `steps` → 知道拿到什么上下文。细节契约见 [`preflight-spi-v1.md`](preflight-spi-v1.md)；管道总览见 [`gateway-solve-preflight.md`](gateway-solve-preflight.md)。

## 两层概念

| 层 | 谁负责 | 内容 |
| --- | --- | --- |
| **平台** | 平台发布 e2b 模板 | 公共运行时（claw、nfs、strict/relaxed 基础工具） |
| **项目** | 每个 `projId` 的业务配置 | `solve_preflight_json.steps[]`：预装软件、MCP 热身、每轮语言等 |

**同一个 worker 模板服务 N 个 project。** 项目预装 / 钩子写在该项目的 preflight 管道里，**不要**按业务 fork 模板。

## 事件表（`steps[].on`）

嵌套：`worker` ⊃ `session` ⊃ `turn`。每层有 `start` / `end`。

| `on` | 何时触发 | 何时不触发 |
| --- | --- | --- |
| `worker.init.start` | 该项目 slot **第一次 create** 成功之后、ready 之前 | 池复用 / 唤醒 |
| `worker.init.end` | init 步骤跑完、upsert ready 之前 | 同上 |
| `worker.reuse.start` | acquire **已有** warm worker（缓存命中） | create 当次 |
| `worker.reuse.end` | 本轮 solve 结束、release 前（预留） | — |
| `session.start` | 该 `sessionId` 第一次进入 solve | 续聊 / 已 satisfied |
| `session.end` | session 关闭/归档（预留） | 不是每轮结束 |
| `turn.start` | 每轮 `push_user_text` 之后、对话之前 | — |
| `turn.end` | 该轮对话结束（SPI 已支持；solve 内调度为后续接线） | — |

时序（简图）：

```text
create sandbox
  → worker.init.start → worker.init.end → slot ready
acquire existing
  → worker.reuse.start → Landlock → session.start? → turn.start → 对话 → turn.end → worker.reuse.end?
```

`worker.init.*` / `worker.reuse.start` 在 **Landlock 之前**（才能装 OS 包）。`session.*` / `turn.*` 在 solve 内、jail 之后。

## 配置怎么写

```json
{
  "steps": [
    {
      "pluginId": "apt_project_tools",
      "on": "worker.init.start",
      "impl": {
        "type": "subprocess",
        "command": ["/claw_ds/.claw/preflight/apt_project_tools.sh"]
      }
    },
    {
      "pluginId": "warm_cache_refresh",
      "on": "worker.reuse.start",
      "impl": { "type": "subprocess", "command": ["/claw_ds/.claw/preflight/warm_cache.sh"] }
    },
    {
      "pluginId": "sqlbot_mcp_start",
      "on": "session.start"
    },
    {
      "pluginId": "turn_language",
      "on": "turn.start"
    }
  ]
}
```

存于 `project_config.solve_preflight_json`，物化到 `home/.claw/solve-preflight.json`。

### 装软件（典型）

1. 写子进程脚本（在 guest 里 `apt-get` / `pip` 等）。
2. Admin **插件库**注册 `pluginId` + 默认 `command`。
3. 项目 Preflight 页加一步：`on = worker.init.start`。
4. **重置 worker**（与 `worker_env_json` 相同：保存不自动 rotate）。

失败只毁掉**该项目** slot，不影响平台模板和其他 project。

## 标准上下文（按事件裁剪）

SPI 请求含 `event` + `context`。**只出现该事件合法字段**。

公共：`projId`、`workerId`、`workDir`、`event`

| 事件 | 额外字段 | 禁止 |
| --- | --- | --- |
| `worker.init.*` | `templateId`、`sandboxId`、`workerProfileMode` | `userPrompt`、`turnId` |
| `worker.reuse.*` | 同上 + 可选 `sessionId` | `userPrompt` |
| `session.*` | `sessionId`、`extraSession` | — |
| `turn.start` | `sessionId`、`turnId`、`userPrompt`、`priorUserPrompts`、`extraSession`、`model`、`isContinuation` | — |
| `*.end` | 对应 start 字段 + `outcome`（`ok`\|`error`）、`durationMs`；`error?` 仅 outcome=error | — |

`worker.init.start` 插件以副作用 + `status=ok|error|skip` 为主；不要依赖 `lockLanguage` 等会话 effect（框架会拒绝）。

## Admin 操作

1. 全局：**Preflight 插件库** → 注册 `pluginId` / 展示名 / 默认 command。
2. 项目：**Preflight** 页 → 添加步骤 → 选插件与 `on` → 保存草稿 / 生效。
3. `worker.init.*`：保存后到 **Worker profile** 使用「重置 worker」。

## 写一个子进程插件

stdin：SPI v1 JSON（含 `event`、`step`、裁剪后的 `context`）。  
stdout：`{"status":"ok"|"skip"|"error","effects":[...],"message"?}`。

最小例子（`worker.init.start`）：

```bash
#!/bin/sh
# apt_project_tools.sh — read SPI from stdin, install tools, print ok
set -euo pipefail
cat >/dev/null
DEBIAN_FRONTEND=noninteractive apt-get update -qq
DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends jq
printf '%s\n' '{"status":"ok","effects":[]}'
```

| status | 含义 |
| --- | --- |
| `ok` | 成功；应用允许的 effects |
| `skip` | 无操作 |
| `error` | 失败；create 路径会销毁该项目 slot |

## 旧 `scope` 兼容

| 旧 `scope` | 映射 `on` |
| --- | --- |
| `every_turn` | `turn.start` |
| `session_first_turn` | `session.start` |

读侧：`on` 优先；同时存在时以 `on` 为准。物化写出 `on`（turn/session start 仍带兼容 `scope`）。

## 常见坑

- 不要在 `turn.start` 装 OS 包（Landlock 之后，strict 会失败）。
- 不要按项目 fork e2b 模板；预装挂 `worker.init.start`。
- init 失败只毁本项目 slot。
- 保存 preflight **不会**自动重建已有 worker。

## 相关

- [`gateway-solve-preflight.md`](gateway-solve-preflight.md)
- [`preflight-spi-v1.md`](preflight-spi-v1.md)
- [`project-config-model.md`](project-config-model.md)
- Schema：[`schemas/preflight-spi-v1.json`](../schemas/preflight-spi-v1.json)

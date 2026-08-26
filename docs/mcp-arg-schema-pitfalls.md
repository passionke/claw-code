# MCP 工具 `inputSchema` 避坑（参数类型）

Author: kejiqing

Harness 在 `tools/call` 前按 discovery 的 `inputSchema` 做**类型硬校验**（不 coerce）。类型不符时直接失败，错误含 `path` / `expected` / `actual` / `(model tool_use)` / `Call not sent.`，便于区分「模型参数错」与「传输/实现 bug」。

## 建议：复杂 `object[]` 不要放在根字段

对照实验（qwen3.7-max）：根级 `items` / `itemList`（`object[]`）易被写成 **JSON 字符串**；同形嵌套在 object 内（如 `payload.itemList`）、根级 `string[]`、builtin `TodoWrite.todos` 则正常。

| 做法 | 建议 |
|---|---|
| 推荐 | `payload.itemList: object[]` 或等价嵌套 |
| 避免 | 根字段直接 `itemList: object[]`（易 string 退化） |

## Harness 行为

- **不**把 string 自动 `JSON.parse` 成 array/object/number
- 无 `inputSchema` 或 schema 非 object：跳过门禁
- `oneOf` / `anyOf` / `allOf` 或联合 `type: [...]`：该子树跳过类型检查（防误伤）

实现：`runtime::validate_mcp_tool_arguments`；出口：`McpServerManager::call_tool` / `call_tool_concurrent`。

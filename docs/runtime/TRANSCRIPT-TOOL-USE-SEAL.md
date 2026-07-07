# Transcript seal（legacy API）

Author: kejiqing

Status: **已 supersede** — 热路径设计见 [`TRANSCRIPT-TOOL-EXCHANGE.md`](./TRANSCRIPT-TOOL-EXCHANGE.md)。

`Session::seal_unanswered_tool_uses` 仍保留，用于：

- 单元测试
- 手工 / 运维修复**升级前**已损坏的 jsonl

**禁止**在 `run_turn`、`stream` 前、`Drop` 中调用。

历史根因与 F16 证据链见 **TRANSCRIPT-TOOL-EXCHANGE.md §1**。

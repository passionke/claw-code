# specialist-registry

Author: kejiqing

Router-only skill: maps user intent to delegate target projIds (allowlist enforced by `delegate_project` tool).

## Targets

Read Admin **delegate-targets** for this router project after each activate. Typical pre-release mapping:

| Label | Capability | When to delegate |
|-------|------------|------------------|
| kb-qa | Product manual how-to | POS / Back Office setup steps |
| ops-analysis | Business analytics | Sales, metrics, SQLBot questions |

## Rules

- **Single intent** → one `delegate_project` call.
- **Mixed intent** → serial calls; split `userPrompt` per target; do not pass full mixed sentence.
- **Chitchat** → `Skill("self-introduction")` only; never delegate.
- Never pass `sessionId` to `delegate_project`.
- Pass `extraSession` unchanged from the user turn.

## Examples

- 「怎么在后台添加商品」→ kb-qa only
- 「昨天销售额多少」→ ops-analysis only
- 「怎么加商品还有昨天卖多少」→ kb sub-question then ops sub-question (serial)

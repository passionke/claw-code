# Gateway system prompt 组装契约

Author: kejiqing

本文档是 **Admin `claude_md` / 物化 / runtime `load_system_prompt`** 的硬性契约。改 `prompt.rs`、`project_config_apply.rs` 或物化路径前**必须先读**，并跑对应单元测试（见文末）。

相关：`docs/project-config-model.md`（PG 真源与路径表）、`GET /v1/project/prompt/{ds_id}/effective`（Admin 预览）。

---

## 1. 设计意图（勿再搞反）

| 配置来源 | 作用 | **不是** |
| --- | --- | --- |
| **`gateway_global_settings.system_prompt_default`** → `.claw/system_prompt_scaffold.md` | 替换内置英文 intro / `# System` / `# Doing tasks` / actions | 项目业务指令 |
| **`project_config.claude_md`** → `CLAUDE.md`（+ 宿主机 `home/CLAUDE.md`） | `# Claude instructions` 段（项目/QueryX 业务规范） | **禁止**再写入 `.claw/system_prompt_user_override.md` 顶替 scaffold |
| **`project_config.mcp_servers_json`** → `.claw/settings.json` | `# Runtime config` 里的 `mcpServers`；runtime 加载 MCP 进程 | **禁止**把 MCP tool schema 塞进 system prompt |
| **`project_config.rules_json`** | `# Project rules` | — |
| **Solve 请求 `extraSession`** | `# Project context` 内 JSON | — |
| **SQLBot preflight（首轮 solve）** | 动态追加 `# SQLBot context`（`gateway_schema_prompt_section`） | **不在** `/effective` 静态预览里 |

**一句话**：`claude_md` 只替换「原来从磁盘读的 CLAUDE.md」，**不**替换 PG 全局 scaffold，**不**吞掉 rules / MCP 配置段 / extraSession。

---

## 2. 静态 system prompt 段顺序（`load_system_prompt`）

固定顺序（中间以 `\n\n` 拼接；`__SYSTEM_PROMPT_DYNAMIC_BOUNDARY__` 为分界）：

1. **Builtin scaffold** — `.claw/system_prompt_scaffold.md`（有则用之；无则 hardcoded 段）。**两者**均受 `auto_hidden_system_prompt`（默认 `1`）与是否存在非空 instruction 文件（如 `CLAUDE.md`）影响：有 instruction 且 auto_hidden 开启时**整段省略**
2. `__SYSTEM_PROMPT_DYNAMIC_BOUNDARY__`
3. **`# Claude instructions`** — 自 `CLAUDE.md` / 祖先链发现（**不是** `system_prompt_user_override.md`）
4. **`# Project rules`** — `.cursor/rules/*.mdc`
5. **`# Environment context`**
6. **`# Project context`** — 含 `extraSession`（solve 传入时）
7. **`# Runtime config`** — 含 `mcpServers`（来自 `.claw/settings.json`）

**首轮 solve 额外**（仅 `gateway-solve-turn`，非 `/effective`）：

8. **`# SQLBot context (preflight, session-local)`** — preflight 物化了 `home/*.md` 时

---

## 3. MCP 与 system prompt 的边界

| 内容 | 在 system prompt？ | 在哪里 |
| --- | --- | --- |
| `mcpServers` 配置 JSON | **是** | `# Runtime config` |
| MCP 工具名 / input schema / 工具说明 | **否** | API `tools` 数组（`DirectApiClient` / MCP bridge） |
| SQLBot 表结构路径提示 | **仅 preflight 后首轮** | 动态段 + 会话内 `home/schema.md` 等文件 |

Admin「系统提示词」页与 clawTap 看到的 **不含** tool schema；缺 MCP **用法**应在 `claude_md` 正文里写，缺 MCP **连接**应查 `# Runtime config`。

---

## 4. 物化路径（`project_config_apply`）

| 字段 | 宿主机 `ds_*` | Pool guest `/claw_host_root` |
| --- | --- | --- |
| `claude_md` | `home/CLAUDE.md` + 根 `CLAUDE.md` | 根 `CLAUDE.md` |
| scaffold（PG 默认） | `.claw/system_prompt_scaffold.md` | 同左 |
| `mcp_servers_json` | `.claw/settings.json`（`apply` 后 `write_ds_settings_json`） | `materialize_in` 写入 |
| `prompt_limits_json` | `.claw/settings.json` → `instructionFileMaxChars` / `instructionTotalMaxChars` | 同左；`{}` 时用默认 8000 / 24000 |
| **已废弃** | ~~`.claw/system_prompt_user_override.md`~~ | **不得**再从 `claude_md` 写入；apply 会 **删除** 遗留文件 |

**`prompt_limits_json`**（Admin「系统提示词」页）：单文件默认 **8000** 字符；`# Claude instructions` 与 `# Project rules` 段各有一份合计默认 **24000**。优先级：项目 settings → `CLAW_INSTRUCTION_*` 环境变量 → 默认。

---

## 5. `auto_hidden_system_prompt`（默认 `1`）

写在 `.claw/settings.json`。当存在非空 instruction 文件（如 `CLAUDE.md`）时：

- **省略** PG `.claw/system_prompt_scaffold.md` **与** hardcoded intro / `# System` / `# Doing tasks` / actions（二者择一有则跳过，无 scaffold 时跳过 hardcoded）

设为 `0` / `false` 可恢复 scaffold（或 hardcoded）与 `CLAUDE.md` 同时出现。

---

## 6. 回归测试（改代码必跑）

| 测试 | 位置 | 保护什么 |
| --- | --- | --- |
| `gateway_system_prompt_assembly_contract` | `runtime/src/prompt.rs` | 全段顺序 + scaffold + claude + rules + MCP config + extraSession；legacy override 不得出现 |
| `legacy_user_override_file_is_ignored` | 同上 | 磁盘上遗留 `system_prompt_user_override.md` 不参与组装 |
| `load_system_prompt_keeps_scaffold_and_claude_instructions_separate` | 同上 | scaffold 与 claude 分工 |
| `build_guest_materialize_writes_includes_claude_and_settings` | `http-gateway-rs/.../project_config_apply.rs` | guest 物化含 `CLAUDE.md` + settings，**无** user_override |
| `build_settings_json_from_row_materializes_prompt_limits` | `project_config_apply.rs` | PG → settings.json 长度键 |
| `instruction_limits_from_settings_json_override_env` | `prompt.rs` | 项目 settings 优先于 env |

```bash
cd rust
cargo test -p runtime gateway_system_prompt_assembly_contract legacy_user_override
cargo test -p http-gateway-rs apply_full_materializes build_guest_materialize_writes_includes_claude
```

---

## 7. 禁止项（review checklist）

- [ ] 不得把 `claude_md` 写回 `system_prompt_user_override.md`
- [ ] 不得让 `read_gateway_user_prompt_override` 重新参与 `load_system_prompt`（路径仅保留 guest lock 兼容）
- [ ] 不得为 scaffold 与 claude 再开第二套 early-return 分叉
- [ ] 不得用 `/effective` 缺 `# SQLBot context` 当 bug（preflight 动态段）
- [ ] 不得把 MCP tool 列表写进 system prompt 代替 API tools

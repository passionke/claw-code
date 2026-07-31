# Rust 网关文档实践

Author: kejiqing

目标：让 Rust 网关文档可维护、可审查、可发布；对外契约从类型推导，少用泛型。

## 1. 文档分层（固定）

- `docs/http-gateway-rs-quickstart.md`
  - 面向使用者：启动、验证、排障
- `docs/http-gateway-rs-api.md`
  - 面向集成方：接口路径和用途
- `deploy/stack/README.md`
  - 面向部署：镜像构建和编排
- `GET /openapi.json` / Swagger UI
  - 面向联调：请求/响应 schema **仅**来自 Rust `utoipa`（见 `rust/crates/http-gateway-rs/src/openapi.rs`）

系统边界不变量（含对外 API 明确类型）：[`boundaries-claw-stack.md`](boundaries-claw-stack.md)。

## 2. 对外契约类型（强约束）

新增或变更对外 HTTP 字段时：

1. 先定义或复用 **明确 Rust 结构 / 闭环枚举**，并挂 `utoipa::ToSchema`。
2. 在 `routes/` 的 handler 上用 `#[utoipa::path]` 引用该类型；由 `openapi.rs` 的 `DerivedApi` 汇总。
3. **禁止**对已有结构类型使用 `#[schema(value_type = Object)]` / `Vec<Object>` 覆盖，以免 Swagger 只剩 `additionalProp*`。
4. **禁止**手写第二份 OpenAPI JSON 与代码并行维护。
5. **例外**：真正无固定形状的开放 JSON bag（`project_config` 的 `*Json`、MCP servers map、工具 `input`、`trace_tail`、`error`/`outputJson` 等）可保留 `Object`，但须在字段注释或 `http-gateway-rs-api.md` 中写明为何不能结构化。

## 3. 变更即更新（强约束）

当修改 `rust/crates/http-gateway-rs/src/routes/`（或 DTO / `openapi.rs`）时，同步检查：

1. 是否新增/删除/改名接口路径
2. 是否改变请求字段或返回关键字段（含枚举、attachments 等）
3. 是否影响部署变量（`CLAW_*`）
4. OpenAPI 是否仍能从类型完整表达（无无理由的泛型 Object）

若有变化，必须同步更新：

- `docs/http-gateway-rs-api.md`
- `docs/http-gateway-rs-quickstart.md`（若影响使用方式）
- 根 `.env` 模板 / `deploy/stack/env.*.example`（若影响部署变量）

## 4. PR 检查清单（建议复制到描述）

- [ ] 路由变更是否已更新 `docs/http-gateway-rs-api.md`
- [ ] 启动命令或环境变量变更是否已更新 quickstart / env 模板
- [ ] OpenAPI 仍从 `ToSchema` / `utoipa::path` 推导；无新增无理由的 `value_type = Object` 覆盖已有结构
- [ ] 本地已验证 `GET /healthz` 与 `GET /openapi.json` 关键 schema
- [ ] 至少验证一次 `solve_async + tasks` 链路
- [ ] MCP 默认联通（`/v1/mcp/injected/{projId}`）可用

## 5. 最小验收命令

```bash
curl -sS http://127.0.0.1:18088/healthz
curl -sS http://127.0.0.1:18088/openapi.json | python3 -c 'import json,sys; d=json.load(sys.stdin); print(d["components"]["schemas"]["SolveAttachment"]["properties"]["kind"])'
curl -sS -X POST http://127.0.0.1:18088/v1/solve_async \
  -H "Content-Type: application/json" \
  -d '{"projId":1,"userPrompt":"smoke"}'
curl -sS http://127.0.0.1:18088/v1/mcp/injected/1
```

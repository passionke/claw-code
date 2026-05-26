# Rust 网关文档实践

目标：让 Rust 网关文档像 Python 网关一样可维护、可审查、可发布。

## 1. 文档分层（固定）

- `docs/http-gateway-rs-quickstart.md`
  - 面向使用者：启动、验证、排障
- `docs/http-gateway-rs-api.md`
  - 面向集成方：接口路径和用途
- `deploy/stack/README.md`
  - 面向部署：镜像构建和编排

## 2. 变更即更新（强约束）

当修改 `rust/crates/http-gateway-rs/src/main.rs` 里路由时，同步检查：

1. 是否新增/删除/改名接口路径
2. 是否改变请求字段或返回关键字段
3. 是否影响部署变量（`CLAW_*`）

若有变化，必须同步更新：

- `docs/http-gateway-rs-api.md`
- `docs/http-gateway-rs-quickstart.md`（若影响使用方式）
- `deploy/stack/.env.example`（若影响部署变量）

## 3. PR 检查清单（建议复制到描述）

- [ ] 路由变更是否已更新 `docs/http-gateway-rs-api.md`
- [ ] 启动命令或环境变量变更是否已更新 quickstart / `.env.example`
- [ ] 本地已验证 `GET /healthz`
- [ ] 至少验证一次 `solve_async + tasks` 链路
- [ ] MCP 默认联通（`/v1/mcp/injected/{dsId}`）可用

## 4. 最小验收命令

```bash
curl -sS http://127.0.0.1:18088/healthz
curl -sS -X POST http://127.0.0.1:18088/v1/solve_async \
  -H "Content-Type: application/json" \
  -d '{"dsId":1,"userPrompt":"smoke"}'
curl -sS http://127.0.0.1:18088/v1/mcp/injected/1
```

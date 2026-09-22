# Admin 账号体系（按 proj_id 空间）

Author: kejiqing

## 角色

| 角色 | 含义 |
|------|------|
| `system_admin` | 本 cluster 全部项目；可创建账号、设定空间成员；可管全局设置 |
| 空间管理员 | `gateway_admin_project_members` 中的账号；仅能管理已加入的 `proj_id` |

一个账号可加入多个 `proj_id`。

## 登录

1. Cluster **零账号**时：用 `PLAYGROUND_ADMIN_USER` / `PLAYGROUND_ADMIN_PASSWORD` seed 一条 `system_admin`。
2. Playground `POST /__admin_login__` → gateway `POST /v1/admin/auth/login` → cookie 存 `cass_…` 会话。
3. `/__proxy__` 自动注入 `Authorization: Bearer cass_…`。

## 我的 TOKEN

Admin UI：**我的 TOKEN**（侧边栏与右上角）。同一页签发三种，明文都只在颁发时返回一次。

- MCP：`GET/POST/DELETE /v1/admin/me/mcp-tokens`，绑定当前账号的 `camt_`。Admin MCP `POST /v1/admin/mcp` 按账号 ACL 校验工具 `projId`。存量无 `accountId` 的 `camt_` 过渡期内视为 system_admin。
- 登录：`GET/POST /v1/admin/me/sessions`、`DELETE /v1/admin/me/sessions/{sessionId}`。与网页登录同一张 `gateway_admin_sessions`，Bearer `cass_`，有效期 7 天。新签发不替换当前浏览器会话。
- 接口调用：`GET/POST /v1/projects/{projId}/model-api-keys`、`DELETE .../{id}`。绑定顶栏当前项目的 `ngmk_`，用于 `/v1/responses` 与 `/v1/chat/completions`。

## 关键 API

- `GET/POST /v1/admin/accounts` — 仅 system_admin（Admin UI：**全局配置 → 账号管理**）
- `PUT/DELETE /v1/admin/accounts/{id}/projects/{projId}` — 设定/移除空间管理员
- `GET /v1/admin/auth/me` — 当前身份与 `projectIds`
- `GET/POST/DELETE /v1/admin/me/mcp-tokens` — 我的 MCP token（`camt_`）
- `GET/POST /v1/admin/me/sessions`、`DELETE /v1/admin/me/sessions/{sessionId}` — 我的登录 token（`cass_`）

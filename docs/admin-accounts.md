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

## MCP Token（持续管理调用）

- 任意登录账号：`POST /v1/admin/me/mcp-tokens` 颁发绑定自身的 `camt_`（明文仅一次）。
- Admin MCP `POST /v1/admin/mcp` 按账号 ACL 校验工具 `projId`。
- 存量无 `accountId` 的 `camt_`：过渡期内视为 system_admin 作用域（兼容旧脚本）。

## 关键 API

- `GET/POST /v1/admin/accounts` — 仅 system_admin（Admin UI：**全局配置 → 账号管理**）
- `PUT/DELETE /v1/admin/accounts/{id}/projects/{projId}` — 设定/移除空间管理员
- `GET /v1/admin/auth/me` — 当前身份与 `projectIds`
- `GET/POST/DELETE /v1/admin/me/mcp-tokens` — 我的 token

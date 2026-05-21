# Gateway async playground（本地 Web）

## 技术栈（测试心智）

- **本目录**：`server.py` 仅用 **Python 3 标准库**（`http.server` + `urllib`），前端为 **单文件 `index.html`**。不建 Rust crate、不跑 `cargo`、**不需要 `pip install`**。
- **被测对象**：你已在跑的 **claw `http-gateway-rs` 网关**（Rust）只是 HTTP 对端；本工具不实现网关逻辑，只做同源代理 + 静态页，方便浏览器里点几下做联调。

用于在浏览器里走通与 `POST /v1/solve_async`（文档中常与 *resolve_async* 语义对齐）相关的链路：

1. 提交异步任务（可选续聊 `sessionId`）
2. 轮询 `GET /v1/tasks/{taskId}`
3. 当 `hasReport === true` 时拉取 `GET /v1/biz_advice_report`（`stream=true`，SSE）

## 为何需要 `server.py`

`http-gateway-rs` 未配置浏览器 CORS（仓库内无 `Access-Control-Allow-Origin`），直接用 `file://` 或任意前端源请求网关会被浏览器拦截。本目录的 **同源小代理** 只转发到固定 allowlist 的主机与端口 **18088**，用于本地调试。

## 运行

```bash
cd web/gateway-async-playground
python3 server.py
```

浏览器打开终端里提示的地址（默认 `http://127.0.0.1:18765/`）。

solve_async 页顶部可配置 **claude-tap Live** 基址（默认 `http://127.0.0.1:3000`）；每轮卡片上的 **session** 点击后打开 `?session=<sessionId>`（与网关 `sessionId` / `taskId` 同值）。

### 项目管理页

- `http://127.0.0.1:18765/admin` — 按 `dsId` 管理项目工作区（数据来自 `/healthz` 的 `projectsGitMirror.dsWorkspaces`）：
  1. **项目**：列表、`POST /v1/init` 初始化
  2. **Skills**：`GET /v1/skills/{ds_id}`、`POST /v1/project/skills/{ds_id}`
  3. **MCP**：`GET/POST/DELETE /v1/mcp/injected|inject`
  4. **Rules**：`GET/POST /v1/project/claude/{ds_id}`（`home/CLAUDE.md`）
  5. **系统提示词**：`GET/POST /v1/project/prompt/{ds_id}/effective`
  6. **Tools**：`GET /v1/project/tools/catalog` + `GET/PUT /v1/project/config/{ds_id}`（`allowedToolsJson`）

PostgreSQL 仅存会话/轮次/反馈；项目配置在 Git 工作区，见页面内说明。

## 预设网关 Base

- `http://127.0.0.1:18088`（localhost）
- `http://192.168.9.252:18088`
- `http://10.200.2.171:18088`

与 `scripts/verify-live-report-flow.sh` 一致，请求体中带 `assistantStreamSpill: true` 与示例 `extraSession`（`tenant_code` / `solution_code` / `biz_type` + 你填的 `store_id` / `org_id`）。若网关未启用 `CLAW_GATEWAY_LIVE_BIZ_REPORT_SPILL=1`，运行中 `hasReport` 可能长期为 `false`，报告仍可能在终态后走润色路径；详见 `docs/http-gateway-rs-api.md`。

Author: kejiqing

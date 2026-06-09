# Gateway async playground（本地 Web）

## 技术栈（测试心智）

- **本目录**：`server.py` 仅用 **Python 3 标准库**（`http.server` + `urllib`），solve 对话在 **`../gateway-admin`** 的 **`/admin/chat`**（Ant Design）；`/` 重定向到该页。`index.html` 仅作遗留参考。不建 Rust crate、不跑 `cargo`、**不需要 `pip install`**（admin 构建用 Node，见 `web/gateway-admin/README.md`）。
- **被测对象**：**claw `http-gateway-rs` 网关**（Rust）只是 HTTP 对端；本工具做同源代理 + 静态页。

用于在浏览器里走通 `POST /v1/solve_async` 相关链路：提交任务 → 轮询 `GET /v1/tasks/{taskId}` → `hasReport` 时 SSE 报告。

## 为何需要 `server.py`

网关未配置浏览器 CORS。本代理只转发到 **allowlist** 主机/端口（见环境变量 `PLAYGROUND_ALLOWED_HOSTS` / `PLAYGROUND_ALLOWED_PORTS`）。

## 本地运行（仅 Python）

```bash
cd web/gateway-async-playground
python3 server.py
```

默认 `http://127.0.0.1:18765/`；`GET /__config__` 返回 `defaultGatewayBase`（本机一条）。Admin 顶栏网关下拉会从默认网关的 **`GET /v1/pools`** 自动列出同 cluster 其它 pool 主机（共享 PG 时无需再配 `PLAYGROUND_EXTRA_GATEWAY_BASES`）；该变量仅作遗留/手工补充。

## 与 gateway 一并 compose 部署

`./deploy/stack/gateway.sh build` 会构建 **`claw-gateway-playground:local`**；`gateway.sh up` 会拉起 **`gateway-playground`**（与 `gateway-rs` 同批 up/down，不含 postgres）。

仓库根 `.env`（见 `.env.example`）：

| 变量 | 作用 |
| --- | --- |
| `GATEWAY_PLAYGROUND_HOST_PORT` | 宿主机端口，默认 `18765` |
| `PLAYGROUND_PUBLIC_GATEWAY_BASE` | 浏览器默认网关，如 `http://127.0.0.1:8088`（与 `GATEWAY_HOST_PORT` 对齐） |
| `PLAYGROUND_GATEWAY_BASE` | 容器内代理上游，默认 `http://gateway-rs:8080` |

访问：

- `http://127.0.0.1:${GATEWAY_PLAYGROUND_HOST_PORT:-18765}/` — solve_async 调试
- `http://127.0.0.1:18765/admin` — 项目管理（需登录；账号密码见根目录 `.env` 的 `PLAYGROUND_ADMIN_USER` / `PLAYGROUND_ADMIN_PASSWORD`，默认 `admin` / `sunmi123`）

连通性：`./deploy/stack/gateway.sh check`（含 playground `/__config__`）。

## 页面说明

- **index**：网关下拉（含 compose 注入的 default preset）；**proj_id** 从 `GET /healthz` 的 `projectsGitMirror.projWorkspaces`（兼容 legacy `dsWorkspaces`）；**claude-tap Live** 从 `GET /v1/gateway/global-settings` 的 `clawTap`；`store_id` / `org_id` 仍可选手填；多轮对话、呼吸灯 poll、`progressHistory`、报告 SSE；session 链接用 `clawTap.liveSessionUrlTemplate`
- **admin**（`web/gateway-admin`，Ant Design）：顶栏 **proj_id** 与 solve 页同源；`GET/POST` 项目、Skills、MCP、CLAUDE.md、Rules、prompt、tools catalog
- 修改 admin UI：`cd web/gateway-admin && npm run build`，提交 `dist/`；旧单页备份 `admin.legacy.html`

多机共享 PG：只需各机 `PLAYGROUND_PUBLIC_GATEWAY_BASE` 指向本机网关；Admin 从 `claw_pool` 自动拼出其它 `poolId · host:port` 选项。仅当某台未注册进 `claw_pool` 时，才在 `.env` 用 `PLAYGROUND_EXTRA_GATEWAY_BASES` 手工补一条。

Author: kejiqing

# Gateway Admin（Vite + React + Ant Design）

项目管理后台，由 `gateway-async-playground` 在 `/admin` 提供静态资源。

Author: kejiqing

## 技术栈

- Vite 5 + React 18 + TypeScript
- Ant Design 5（暗色主题）
- 构建产物 **`dist/` 提交进 Git**（镜像/compose **不**跑 `npm build`）

## 标准构建（推荐）

在**仓库根目录**：

```bash
./deploy/stack/gateway.sh quick          # 日常：含 admin dist + playground 镜像
./deploy/stack/gateway.sh admin-build    # 仅重建 admin dist
./deploy/stack/gateway.sh playground     # host 调试页（会先 admin-build）
./deploy/stack/gateway.sh build          # 全量镜像前也会 build admin
```

`dist/` **提交进 Git**；镜像/compose **不**在容器里跑 `npm`（见 `Containerfile.gateway-playground`）。

跳过构建（仅用仓库里已有 dist）：`SKIP_GATEWAY_ADMIN_BUILD=1 gateway.sh playground`

## 本地前端开发（可选）

```bash
cd web/gateway-admin
npm ci
npm run dev    # Vite 开发服；登录/代理需自行对接
npm run build
```

`.npmrc` 默认 `registry.npmmirror.com`。

旧版单文件 UI：`web/gateway-async-playground/admin.legacy.html`。

## 目录

- `src/api/client.ts` — `__proxy__` / `__config__` / 登录
- `src/context/AppContext.tsx` — 网关、proj_id、项目列表
- `src/pages/*` — 各 Tab（对应原 admin.html）

# Gateway Admin（Vite + React + Ant Design）

项目管理后台，由 `gateway-async-playground` 在 `/admin` 提供静态资源。

Author: kejiqing

## 边界（单一默认路径）

| 层 | 职责 |
|----|------|
| **Git** | 只提交 `src/`，**不提交** `dist/` |
| **构建** | CI / `Containerfile.gateway-playground` 内 `npm ci && vite build` |
| **运行时默认** | playground **镜像**内 `/app/admin-dist`（`gateway.sh up` / `quick`） |
| **前端热更新（可选）** | `admin-build` 后 `CLAW_GATEWAY_ADMIN_BIND=1 gateway.sh up` bind 本机 `dist/` |

不要用「Git 里半截 index.html + 本机旧 assets + 默认 bind」——会黑屏。

## 常用命令

```bash
./deploy/stack/gateway.sh quick          # 推荐：完整 playground 镜像 + up
./deploy/stack/gateway.sh up             # admin 来自已有 playground 镜像
./deploy/stack/gateway.sh build local    # 含 playground 镜像（镜像内 npm build）
```

改 React 源码后（可选 bind 热更新）：

```bash
CLAW_GATEWAY_ADMIN_LOCAL_BUILD=1 ./deploy/stack/gateway.sh admin-build
CLAW_GATEWAY_ADMIN_BIND=1 ./deploy/stack/gateway.sh up
```

## 本地前端开发（Vite dev server）

```bash
cd web/gateway-admin
npm ci
npm run dev
```

`.npmrc` 默认 `registry.npmmirror.com`。

旧版单文件 UI：`web/gateway-async-playground/admin.legacy.html`。

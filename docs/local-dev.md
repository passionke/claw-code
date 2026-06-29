# 本地开发（懒人版）

Author: kejiqing

## 推荐路径（FC + 外连 PG）

```bash
cp deploy/stack/env.selfhosted-e2b.example .env   # 编辑 CLAW_CLUSTER_ID、FC keys、PG URL
./deploy/stack/gateway.sh quick
```

前提：e2bserver 与 PostgreSQL 已就绪（见 [`architecture-governance.md`](architecture-governance.md)）。

## 一条命令（macOS 全本地 compose）

在**仓库根目录**：

```bash
./deploy/stack/gateway.sh quick
```

会做：

1. **`web/gateway-admin`**：`npm ci && vite build` → `dist/`
2. 快速重建 `claw-gateway-playground` 镜像
3. `up` → `check`（**无** host pool-daemon）

`CLAW_INTERACTIVE_BACKEND` / `CLAW_SOLVE_ISOLATION` 须为 **`fc`**（`env-profile.sh` 默认）。

## 只改根目录 `.env`

```bash
./deploy/stack/gateway.sh up
```

会 `source .env` 并 `--force-recreate` gateway 容器。**不必**为改 env 单独 `pack-deploy`。

## 改 Rust 网关后

```bash
./deploy/stack/gateway.sh pack-deploy
```

## 其它命令

| 命令 | 作用 |
|------|------|
| `./deploy/stack/gateway.sh playground` | 仅起 host 调试页 |
| `./deploy/stack/gateway.sh admin-build` | 只构建 React Admin `dist/` |
| `./deploy/stack/gateway.sh down` | 停 gateway + playground |
| `./deploy/stack/gateway.sh ps` | 看容器 |
| `./deploy/stack/gateway.sh help` | 帮助 |

实现脚本在 `deploy/stack/lib/`；**不要**在 `rust/` 子目录里直接跑 `gateway.sh`。

## 磁盘清理

| 路径 | 清理 |
|------|------|
| `rust/target/debug` | `gateway.sh clean --debug-only` |
| `rust/target` 全部 | `gateway.sh clean` |
| `deploy/stack/.linux-artifacts` | 随 `clean` 删除 |

## 常见坑

- **`zsh: no such file or directory: ./deploy/stack/gateway.sh`** — 先 `cd` 到仓库根。
- **solve 503 / FC 错误** — 查 `CLAW_FC_API_URL`、模板是否已 build；见 `deploy/fc-sandbox/README.md`。
- **Admin 界面旧** — `gateway.sh admin-build` 或 `quick`；浏览器强制刷新。

更多：`deploy/stack/README.md`、`docs/README.md`。

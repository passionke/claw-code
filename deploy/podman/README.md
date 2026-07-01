# `deploy/podman/` — 兼容入口（请用 `deploy/stack`）

Author: kejiqing

**华山一条道**：日常与文档只认 **`./deploy/stack/gateway.sh`**（实现均在 `deploy/stack/lib/`）。

| 场景 | 做法 |
| --- | --- |
| **本地开发** | 根目录 `.env` + **`CLAW_DEPLOY_PROFILE=local`**（强制 **podman**）+ **`./deploy/stack/gateway.sh quick`**；改 Rust/网关镜像用 **`pack-deploy`**（接受编译慢，换稳定）。 |
| **线上** | 根目录 `.env` 按 `deploy/stack/env.production.docker.example`：**`CLAW_CONTAINER_RUNTIME=docker`** + **`./deploy/stack/gateway.sh up --release release-v…`**，**只拉 CI 打的镜像**，不在服务器上 `cargo build` 网关。 |

本目录下的 `*.sh` 仅为**旧路径兼容**，内部 **`exec`** 到 `deploy/stack/lib/` 同名逻辑；新脚本与排障请只看：

- `docs/local-dev.md`
- `deploy/stack/README.md`
- `docs/env-files.md`

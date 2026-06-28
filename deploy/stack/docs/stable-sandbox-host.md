# 稳定沙箱主机（10.22.28.94）

> **DEPRECATED（2026-06）** 宿主机 pool / `stable-dev-up` 已移除。请用 **FC + 外连 PG**：[`docs/architecture-governance.md`](../../../docs/architecture-governance.md)、`env.selfhosted-e2b.example`。

Author: kejiqing

与 GitLab CI **隔离**的 dev-stable：PG `5434` / pool `9954` / tap 代理 `8081` / tap Live `3001`。

> **角色**：本机是 **PG + pool + tap 的托管方**（给模式 B 或直连 Admin 用）。  
> **性能**：Mac 用模式 B 连本机时 **可以工作，但 solve 明显慢于 Mac 全本地栈**（跨网 materialize + pool RPC）。日常开发 Rust/solve 请用 [`env.local.example`](../env.local.example)；详见 [`local-dev-remote-backend.md`](local-dev-remote-backend.md#结论先看)。

## 正路（只在 94 上操作，不要从 Mac rsync 代码）

```bash
ssh sunmax@10.22.28.94
cd ~/work
git clone git@code.sunmi.com:minidata/claw-code.git claw-code-dev-stable   # 首次
# 已有则：cd claw-code-dev-stable && git fetch && git checkout pool_outside && git pull

cd claw-code-dev-stable
cp deploy/stack/env.stable-dev-host.example .env.dev-stable
# 可选：编辑 OPENAI_API_KEY / UPSTREAM_OPENAI_BASE_URL

./deploy/stack/gateway.sh build local          # 在 94 上用 docker 编译，不拷代码
export CLAW_STABLE_DEV_ENV_FILE=$PWD/.env.dev-stable
./deploy/stack/gateway.sh stable-dev-up
```

本机 Mac（模式 B，**可选、非默认**）：

```bash
cp deploy/stack/env.local-remote-backend.example .env
./deploy/stack/gateway.sh pack-deploy local && ./deploy/stack/gateway.sh up
```

模式 B 仅 gateway 在 Mac；solve 走远端 pool，**性能差于全本地**。Mac 与 94 **须同版本** `rust/` 镜像。

## 端口

| | CI | dev-stable |
|--|-----|------------|
| PG | 5433 | **5434** |
| pool | 9944 | **9954** |
| tap 代理 | 8080 | **8081** |
| tap Live | 3000 | **3001** |
| cluster | sunmi-ci-01 | **dev-stable** |

## 验收

```bash
curl -fsS http://127.0.0.1:9954/healthz/live-report
curl -fsS http://127.0.0.1:8081/healthz
curl -fsS http://127.0.0.1:3001/
docker exec claw-dev-stable-postgres psql -U claw_gateway -d claw_gateway \
  -c "SELECT pool_id, advertise_ip FROM claw_pool WHERE pool_id='pool-dev-stable';"
```

## 注意

- **不要 rsync** Mac 仓库到 94；用 git pull。
- pool RPC 无鉴权，仅内网。
- LLM 可在本机首次 `gateway.sh up` 时写入 dev-stable PG（Admin 全局推理）。

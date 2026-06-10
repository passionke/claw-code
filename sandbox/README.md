# claw-sandbox

Author: kejiqing

**独立沙箱执行服务**：容器 worker 池、PG materialize/readback、live stdout 采集。

从 `claw-pool-daemon` 演进；Gateway 通过 HTTP/RPC 调用，不再同进程/同脚本树强绑定。

## 文档

- [系统详细设计（系分）](docs/system-design.md)
- [索引（仓库根）](../docs/claw-sandbox-system-design.md)

## 快速开始

```bash
# 在仓库根目录
./sandbox/deploy/sandbox.sh build
./sandbox/deploy/sandbox.sh up          # P0：等价 pool-daemon-up，二进制为 claw-sandbox
./deploy/stack/lib/admin-solve-e2e.sh 1 ping
```

## Crate

| Crate | 说明 |
|-------|------|
| `claw-sandbox-protocol` | Gateway ↔ Sandbox 共享类型与 OpenAPI |
| `claw-sandbox-server` | 服务二进制 `claw-sandbox` |

## 阶段

- **P0**（当前）：项目骨架 + protocol 抽离 + `claw-sandbox` 二进制（逻辑同 pool daemon）
- **P1**：单 daemon 双 isolation quota
- **P2**：Push SSE，Gateway 不再 proxy pool IP
- **P3+**：Gateway 与 deploy 完全解耦

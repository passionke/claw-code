# claw-sandbox 系分索引

Author: kejiqing

**完整系统详细设计**见：[sandbox/docs/system-design.md](../sandbox/docs/system-design.md)

**Gateway 对沙箱的用法（RPC、边界、一轮 solve 顺序）**：[sandbox/docs/gateway-integration.md](../sandbox/docs/gateway-integration.md)

**项目根目录**：`sandbox/`（workspace + deploy + protocol crate）

**快速开始**：

```bash
# 构建
./sandbox/deploy/sandbox.sh build

# 启动（P0：复用 pool-daemon 部署链，二进制为 claw-sandbox）
./sandbox/deploy/sandbox.sh up

# 验收（与 pool 相同）
./deploy/stack/lib/admin-solve-e2e.sh 1 ping
```

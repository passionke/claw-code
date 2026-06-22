# e2b 自建流量路由问题（F14）— 请 e2b 侧排查

Author: kejiqing  
Date: 2026-06-20  
Claw 集成方：claw-code（Gateway + FC OVS singleton + warm worker）  
e2b 节点：`10.8.0.9`（self-hosted e2bserver，WireGuard `10.8.x`）

---

## 1. 一句话

沙箱**内部**服务（ttyd `:7681`、openvscode `:3000/ovs/`）已正常监听；但从集群外按 E2B SDK 约定的 `{port}-{sandboxId}.{domain}` 访问时，**nginx 返回默认站页面**（`<title>君子慎独</title>`），**未转发进对应 sandbox**。

---

## 2. 期望行为（E2B SDK 契约）

[`e2b.connection_config.get_host`](https://github.com/e2b-dev/e2b) 约定：

```text
{port}-{sandboxId}.{sandbox_domain}
```

本环境配置：

| 项 | 值 |
|----|-----|
| API | `http://10.8.0.9:3000` |
| envd / SDK connect | `http://10.8.0.9:3002` |
| `CLAW_FC_DOMAIN` | `10.8.0.9` |
| worker 端口 | `7681`（ttyd） |
| OVS 端口 | `3000`（openvscode，`--server-base-path=/ovs`） |

示例（sandbox `sbx_448215e49df6`）：

| 用途 | 期望 Host | 期望路径 |
|------|-----------|----------|
| worker ttyd | `7681-sbx_448215e49df6.10.8.0.9` | `/` 或 `/ws`（WebSocket） |
| OVS IDE | `3000-sbx_9d5520516f1a.10.8.0.9` | `/ovs/` |

从 Mac / Gateway 容器经 **nginx 流量入口（:80 或等价）** 用 `Host` 或 DNS `--resolve` 访问上述 Host，应打到沙箱内对应端口。

---

## 3. 实际行为（证据）

### 3.1 沙箱内 — 正常

在 `claw-worker` sandbox 内执行：

```bash
curl -sS -o /dev/null -w '%{http_code}' http://127.0.0.1:7681/    # → 200（ttyd）
```

在 `claw-ovs` sandbox 内：

```bash
curl -sS http://127.0.0.1:3000/ovs/ | head   # → openvscode HTML（非默认站）
```

`terminal/start` 后 `ttyd.pid` 存在，进程监听 `7681`。

### 3.2 集群外 — 打到 nginx 默认站

从 Mac（VPN `10.8.0.2`）执行：

```bash
# OVS
curl -sS --resolve "3000-sbx_9d5520516f1a.10.8.0.9:80:10.8.0.9" \
  "http://3000-sbx_9d5520516f1a.10.8.0.9/ovs/" | grep -o '<title>[^<]*</title>'
# → <title>君子慎独</title>

# worker ttyd
curl -sS --resolve "7681-sbx_448215e49df6.10.8.0.9:80:10.8.0.9" \
  "http://7681-sbx_448215e49df6.10.8.0.9/" | grep -o '<title>[^<]*</title>'
# → <title>君子慎独</title>
```

同 Host 头打 `:3002`、`:3000` 直连亦无法得到沙箱内容（`:3002` 为 envd/exec，非 traffic proxy）。

### 3.3 对 Claw 的影响

| 链路 | 现象 |
|------|------|
| `GET /ovs/workspace` → 浏览器打开 `ovsUrl` | 外网 URL 打开是默认站，非 IDE |
| Gateway `agent/ws` → ttyd | `connect ws://10.8.0.9:80/ws` 得 HTML 200，非 WS 101 |
| E2E `verify-fc-ovs-e2e.sh` | 在检查 OVS body 时失败：`OVS URL hit e2b default site (F14)` |

**结论（有证据链）：** 问题在 **e2bserver 流量入口 / nginx 对 port-prefix 域名的路由**，不是 Claw gateway 业务逻辑。

---

## 4. 复现步骤（给 e2b 同事）

**前提：** VPN 可达 `10.8.0.9`，API Key 有效，已有运行中 sandbox。

```bash
# 1. 列出现有 sandbox
curl -sS "http://10.8.0.9:3000/sandboxes" -H "X-API-Key: ${E2B_API_KEY}"

# 2. 任选一个 claw-worker，在沙箱内确认 ttyd
#    （用 e2b SDK Sandbox.connect + commands.run）
curl http://127.0.0.1:7681/   # 期望 200

# 3. 从集群外测 traffic Host（替换 SANDBOX_ID）
curl -sS --resolve "7681-SANDBOX_ID.10.8.0.9:80:10.8.0.9" \
  "http://7681-SANDBOX_ID.10.8.0.9/" | grep '<title>'
# 实际：<title>君子慎独</title>
# 期望：ttyd 页面或 101 WebSocket upgrade，而非默认站

# 4. OVS sandbox 同理（替换 OVS_SANDBOX_ID）
curl -sS --resolve "3000-OVS_SANDBOX_ID.10.8.0.9:80:10.8.0.9" \
  "http://3000-OVS_SANDBOX_ID.10.8.0.9/ovs/" | grep -iE 'openvscode|vscode|君子慎独'
# 实际：君子慎独
```

---

## 5. 建议 e2b 侧检查项

1. **nginx / ingress** 是否配置了 `{port}-{sandboxId}.{domain}` → 对应 MicroVM 端口的反向代理（与阿里云 FC 公网域名行为对齐）。
2. **`domain` 字段**：创建 sandbox API 返回 `"domain":"localhost"`；SDK 侧我们用 `CLAW_FC_DOMAIN=10.8.0.9` 覆盖。流量路由是否应统一认 `10.8.0.9`？
3. **traffic_access_token**：创建响应含 `trafficAccessToken`；公网代理是否要求额外鉴权头？当前外网请求未带 token 时是否应 401 而非默认站 200？
4. **端口矩阵**：确认 traffic 入口端口（:80 / :443 / 其他）与 envd `:3002` 职责分离文档。

---

## 6. Claw 侧已做（无需 e2b 改）

- Gateway 编排、warm pool、OVS singleton 创建与沙箱内启动脚本 — **沙箱内 OK**
- `fc-sandbox-cleanup.sh` 清理 orphan sandbox
- 验收脚本不再用「HTTP 200」误判，会检测 `<title>君子慎独</title>`

修通 F14 后，Claw 将复跑：

```bash
./deploy/stack/lib/verify-fc-ovs-e2e.sh
```

---

## 7. 参考

- Claw 设计：`docs/ovs-chat/FC-OVS-SINGLETON-DESIGN.md`
- 完整失败表：`docs/ovs-chat/FC-OVS-E2E-FAILURES.md`（F14 行）
- e2b SDK `get_host(port)` → `{port}-{sandboxId}.{sandbox_domain}`

---

## 8. 联系

集成方：kejiqing / claw-code  
环境：Mac dev Gateway + `10.8.0.9` self-hosted e2b + `10.8.0.8` NAS（NAS 挂载另案 F2，与本次 traffic 路由无关）

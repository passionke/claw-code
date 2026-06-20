# FC OVS E2E — 失败尝试记录

Author: kejiqing  
用途：避免重复踩坑；每条必须有**证据**才标为已修复/已验证。

---

## 状态图例

| 标记 | 含义 |
|------|------|
| **OPEN** | 未修复或未复验 |
| **FIXED** | 代码/脚本已改，待 E2E 绿 |
| **VERIFIED** | 本机/10.8.0.9 复验通过 |

---

## 记录表

| # | 现象 | 根因（证据） | 修复 | 状态 |
|---|------|-------------|------|------|
| F1 | `mount.nfs4: multiple version options not permitted` | mount 同时写 `vers=4.2` 与 `nfsvers=4.2` | `fc_interactive_materialize.rs` 只保留 `vers=4.2,_netdev` | FIXED |
| F2 | `mount.nfs4: Operation not permitted`（sandbox 内 root mount） | `CapEff=0000000000000000`；Firecracker 无 `CAP_SYS_ADMIN` | exec mount 失败 → warn + 本地目录 fallback；`nasConfig` 在 create 时带上（e2b 侧是否生效待 V2） | FIXED（fallback）；NAS 真挂 **OPEN** |
| F3 | `mkdir: cannot create directory '/claw_ds/.claw': Permission denied` | mount 失败后 `sudo mkdir` 属 root，user 无法写 | mount 失败分支加 `sudo chown $(id -u):$(id -g)` | FIXED |
| F4 | e2b 累积 **12** 个 orphan sandbox | gateway 重启丢内存池；`stop_session` 未杀沙箱；E2E 只 create 不 cleanup | `fc-sandbox-cleanup.sh`；`shutdown_all`/`FcOvsSingleton::shutdown`；gateway SIGTERM 钩子；E2E 前后计数 + `terminal/stop` | FIXED（cleanup 已验证 12/12 kill）；shutdown 钩子 **待 VERIFIED** |
| F5 | `Could not resolve host: 3000-sbx_….10.8.0.9` | self-hosted domain=IP，子域不 DNS 解析 | `verify-fc-ovs-e2e.sh` 用 `curl --resolve host:80:10.8.0.9` | FIXED |
| F5b | OVS「可达」误判 HTTP 200 | `--resolve` 打到 e2b **默认站** `<title>君子慎独</title>`，非 openvscode | 验收改查 title/关键字，不能只看 status | FIXED（脚本） |
| F6 | `terminal/start` → `timeout waiting for idle Strict worker` | `CLAW_INTERACTIVE_BACKEND=fc` 仍走 per-proj `worker_isolation_json` → podman | `interactive_backend_for_proj` 在 `interactive_backend_is_fc()` 时强制 fc | FIXED |
| F7 | `terminal/start` → `already active for this session` | E2E 重跑未 `terminal/stop`；stop 用错 body（需 query `projId`） | E2E 先 `terminal/stop?projId=` | FIXED |
| F8 | `verify-ovs-claw-e2e.sh` `bad substitution` `${PROMPT@Q}` | macOS bash 3.2 无 `@Q` | heredoc 改 env 传参 + `<<'PY'` | FIXED |
| F9 | agent WS `connect ttyd … HTTP 400/200` | 曾误判为缺 Host / 错端口；**根因 F14 + Mac podman `container_ip` 不可达** | agent 补 Host + token；e2b traffic 改 `127.0.0.1` 端口映射 | **VERIFIED** |
| F14 | **e2b 流量入口未路由进 sandbox** | 修前：外网 → 君子慎独；修后：`401`（无 token）/ `200` openvscode（有路由） | e2bserver traffic :3001 + nginx `e2b-traffic.conf` + `sandbox_domain=10.8.0.9` + podman 发布 7681/3000 到 127.0.0.1 | **VERIFIED** |
| F13 | 同 proj 重复 warm worker（4 sandbox） | gateway 重启丢内存池，e2b 上旧 warm 仍在；`warm_one` 再 create | E2E 前 `fc-sandbox-cleanup.sh` + restart gateway；长期：启动时 reconcile 或 shutdown 杀光 | OPEN |
| F10 | gateway build `uid_args[@]: unbound variable` | bash 空数组展开 | `linux-compile.sh` `${uid_args[@]+"${uid_args[@]}"}` | FIXED |
| F11 | `main.rs` move `state` 编译失败 | shutdown 闭包需 `pool_clients` 但 `state` 已 move 进 axum | `.with_state(state.clone())` | FIXED |
| F12 | bootstrap 预热 proj_1+2 → 稳定 **3** sandbox | 设计：`min_idle=1` × 2 proj + 1 OVS singleton | 非 bug；E2E 应断言「≤ cap」而非 0 | **BY DESIGN** |

---

## 当前验收标准（一条链路）

```bash
# 1. 可选：清 orphan（会 kill 全部 e2b sandbox）
./deploy/stack/lib/fc-sandbox-cleanup.sh
./deploy/stack/gateway.sh up   # 或 podman restart claw-gateway-rs

# 2. 全链路（默认先 cleanup + restart gateway）
./deploy/stack/lib/verify-fc-ovs-e2e.sh

# 3. 仅复跑不测 cleanup
CLAW_FC_E2E_CLEANUP=0 ./deploy/stack/lib/verify-fc-ovs-e2e.sh
```

**通过条件：**

1. `GET …/ovs/workspace` → `ovsBackend=fc`，`ovsUrl` HTTP 200（`--resolve`）
2. `terminal/start` → `workerName` 形如 `fc:sbx_…`
3. `agent/ws` host 侧 `ping` → `OK`
4. `terminal/stop` → `ok:true`
5. sandbox 数量：bootstrap 后约 3（1 OVS + proj_1 + proj_2 warm）；stop 后仍保留 warm idle（不杀 worker）

---

## 待验证假设（不能当结论）

| ID | 假设 | 怎么证伪/证实 |
|----|------|----------------|
| V-NAS | e2b `nasConfig` 在 10.8.0.9 由**宿主机**挂 NAS，sandbox 内可见 `/claw_ds` | create 带 nasConfig 后 `mountpoint /claw_ds`；当前 API 接受但容器内无目录 → **未证实** |
| V-SHUTDOWN | gateway `podman stop` 后 warm+OVS sandbox 被 kill | stop 前后 `GET /sandboxes` 计数 |

---

## 每次 E2E 运行日志（手工追加）

| 时间 | 命令 | 结果 | 备注 |
|------|------|------|------|
| 2026-06-20 | `fc-sandbox-cleanup.sh` | OK 12/12 killed | 清 orphan |
| 2026-06-20 | `verify-fc-ovs-e2e` (cleanup=0) | FAIL F9 agent WS 400 | 根因：:3002 非 traffic proxy |
| 2026-06-20 | `verify-fc-ovs-e2e` (full cleanup) | FAIL F9 `HTTP 200 OK`（非 101） | ttyd 改 :80；仍命中 F14 默认站 |
| 2026-06-20 | 手工 curl title 对比 | **F14 确认** | 外网 Host/`--resolve` → 君子慎独；沙箱内 127.0.0.1 正常 |
| 2026-06-20 | e2b 宣称 traffic proxy 已补（page-cf0c8633）后复验 | **FAIL F14** | 见下「F14 复验 2026-06-20」 |
| 2026-06-20 | 部署 traffic + podman 端口映射 + gateway 重建 | **PASS** | `verify-fc-ovs-e2e: OK`（OVS 200 + agent WS + terminal/stop） |
| _下次跑完填这里_ | | | |

### F14 复验 2026-06-20（e2b 文档称已修，Claw 侧实测 — 部署前）

**结论：代码/文档有了，`10.8.0.9` 运行时未就绪 → F14 仍 OPEN。**

| 检查项 | 命令 | 期望 | 实测 |
|--------|------|------|------|
| traffic 健康 | `curl http://10.8.0.9:3001/traffic-health`（Mac + `podman exec claw-gateway-rs`） | `ok` | **连接被拒**（:3001 无监听） |
| API domain/token | `POST /sandboxes` create `claw-worker` | `domain=10.8.0.9`，含 `trafficAccessToken` | `domain: "localhost"`，**无** `trafficAccessToken` |
| OVS 外网 | `curl --resolve 3000-sbx_15bac74808bf.10.8.0.9:80:10.8.0.9 http://…/ovs/` | openvscode | `<title>君子慎独</title>` |
| Host 直打 :80 | `curl -H 'Host: 3000-sbx_….10.8.0.9' http://10.8.0.9/ovs/` | 非默认站 | 仍 **君子慎独** → nginx traffic 块未生效 |
| 全链路 | `./deploy/stack/lib/verify-fc-ovs-e2e.sh` | 绿 | **FAIL** `OVS URL hit e2b default site` |

**e2b 侧待完成（Affine page-cf0c8633 §4）：**

1. `10.8.0.9` 部署新 `e2bserver` 二进制，`sandbox_domain=10.8.0.9`，日志含 `e2b-traffic listening on 0.0.0.0:3001`
2. nginx 接入 `scripts/nginx-traffic.conf` 并 `nginx -s reload`
3. 复验 `:3001/traffic-health` + create 响应 `domain`/`trafficAccessToken` 后再 ping Claw 跑 E2E

### F14 修复 2026-06-20（已部署 + E2E 绿）

**e2b 侧（`10.8.0.9` / `~/work/e2bserver`）：**

1. `config/default.toml` → `sandbox_domain = "10.8.0.9"`
2. `scripts/nginx-traffic.conf` → `/usr/local/etc/nginx/servers/e2b-traffic.conf` + reload
3. `e2bserver run` 监听 `:3001`（`curl http://127.0.0.1:3001/traffic-health` → `ok`）
4. **Mac podman 补丁**：sandbox create 额外发布 `127.0.0.1:{base+1}:7681`、`{base+2}:3000`；traffic 走 `127.0.0.1` 而非 `container_ip`（`10.88.x.x` 从宿主机不可达）

**Claw 侧：**

- `claw-fc-sandbox-client`：self-hosted create 带 `secure: false`；`traffic_url()` 辅助
- gateway：ttyd WS 支持 `X-Access-Token` / `?token=`
- `deploy/stack/lib/setup-e2b-traffic-proxy.sh`：一键部署 e2b traffic
- `verify-fc-ovs-e2e.sh`：`CLAW_OVS_E2E_FAST=1` 避免 agent WS 等完整 claw 回合

**验收：**

```bash
./deploy/stack/lib/verify-fc-ovs-e2e.sh
# verify-fc-ovs-e2e: OK
```

# e2b 自建流量入口：跨 sandbox 请求体 ≥16KB 概率性 502/431（F15）— 请 e2b 侧排查

Author: kejiqing
Date: 2026-07-01
Claw 集成方：claw-code（Gateway + e2b observe singleton「claude-tap」+ warm worker）
e2b 节点：`10.8.0.1`（self-hosted e2bserver；API `:3000`，envd/connect `:3002`），`domain=supone.top`

---

## 1. 一句话

从一个 sandbox 经 e2b 流量入口 `{port}-{sandboxId}.{domain}` 访问另一个 sandbox 内的 HTTP 服务时，**请求体（request body）≥ 约 16KB 就会被流量入口在转发到目标 sandbox 之前直接拒绝**（瞬时 `502 Bad Gateway`，更大时 `431 Request Header Fields Too Large`），且呈**概率性**（同样大小时成时败）。同样的请求在目标 sandbox 内走 `127.0.0.1` loopback（不经流量入口）则 **16KB–500KB 全部 100% 成功**。

因此问题定位在 **e2b 流量入口（traffic proxy / nginx 反代）对跨 sandbox 请求体的缓冲/限制**，与上游业务服务（claude-tap）无关。

---

## 2. 拓扑与两条链路

```
业务链路（出问题）：
  worker sandbox(claw)  ──HTTP POST──▶  e2b 流量入口  ──▶  observe sandbox(claude-tap:8080)  ──▶ 上游 LLM
                                       8080-{sbx}.supone.top        （目标服务）

对照链路（正常）：
  observe sandbox 内  ──HTTP POST──▶  127.0.0.1:8080(claude-tap)  ──▶ 上游 LLM
  （loopback，完全不经 e2b 流量入口）
```

- worker：`sbx_5ae7e7b46477`（template `claw-worker`）
- observe：`sbx_cf0110c8fce9`（template `claw-observe`，内部 `claude-tap` 监听 `:8080`）
- 流量入口 Host：`8080-sbx_cf0110c8fce9.supone.top`（与 F14 同一入口）

---

## 3. 证据

### 3.1 实验 A — 经 e2b 流量入口（worker → `8080-{sbx}.supone.top`）

同一请求按 `system` 字段填充不同大小的 body，`stream=false`、`max_tokens=1`，每个大小**重复 8 次**，记录 HTTP 状态码与耗时：

| body 大小 | 成功率 | 现象 |
|----------|--------|------|
| 1KB  | **8/8** | 全部 `200`，耗时 5–9s（请求真正到达上游 LLM）|
| 16KB | **1/8** | 7×`502`（0.4s）+ 1×`200`（8.3s）— **概率性** |
| 32KB | **0/8** | 8×`502`（0.4–0.5s）|
| 48KB | **0/8** | 8×`502`（0.5s）|
| 64KB | **0/8** | 8×`431`（0.34s）|
| 100KB–1MB | 0 | `431`（前次取证）|
| 3MB  | 0 | `502`（前次取证）|

特征：**失败一律 < 0.5s 瞬拒**（未达上游），**成功一律 5–9s**（真正打到上游 LLM）。

### 3.2 实验 B —（对照）目标 sandbox 内 loopback，绕过流量入口

完全相同的 body，在 observe sandbox 内对 `127.0.0.1:8080` 发起，每个大小重复 4 次：

| body 大小 | 成功率 |
|----------|--------|
| 16KB  | **4/4** `200` |
| 32KB  | **4/4** `200` |
| 48KB  | **4/4** `200` |
| 64KB  | **4/4** `200` |
| 100KB | **4/4** `200` |
| 500KB | **4/4** `200` |

**结论：唯一变量是「是否经过 e2b 流量入口」。** 上游服务（claude-tap）可稳定处理至少 500KB 的 body；一旦经流量入口，≥16KB 即大面积失败。

### 3.3 失败请求从未到达目标服务

claude-tap 对每个收到的请求都会打印 `→ POST /chat/completions ... ← 200/err` 日志。核对其日志：

- 实验 A 中**成功**的请求（如 8KB/48KB 的那几次）→ tap 日志有完整 `→ ... ← 200` 记录；
- 实验 A 中**失败**（502/431）的请求 → tap 日志**零记录**。

即：`502/431` 由**流量入口自身**返回，请求根本没被转发进目标 sandbox。这排除了 claude-tap（aiohttp）层面 `431/413` 的可能。

### 3.4 业务侧真实故障（首次触发本次排查）

一次真实 solve turn（worker `sbx_5ae7e7b46477`）的 trace：

```
turn_started  ts=1782836039136
turn_failed   ts=1782836498069
  error="api failed after 9 attempts: api returned 502 Bad Gateway:
         read worker relay response: Connection reset by peer (os error 104)"
  iteration=1
```

该 turn 的 LLM 请求体含完整 system prompt + 工具 schema（必然 > 16KB），落入上述失败区间，**9 次重试全部 502**，整个 turn 失败。对照同会话中 body 很小的「语言推断」请求（约 in=121 tokens）则成功——与 3.1 的大小相关性完全吻合。

---

## 4. 复现步骤（e2b 可独立执行）

前提：能解析/访问 `10.8.0.1`，有效 API Key，已有两个运行中的 sandbox（任一在 `:PORT` 监听 HTTP 即可，无需 LLM）。

> 下面用 claude-tap 的 `:8080` 作为目标服务；e2b 也可在 sandbox 内起任意 HTTP echo 服务替代，效果一致——关键只看流量入口是否把不同大小的 body 转发进去。

```bash
# A) 经流量入口：从另一台可达客户端（或 worker sandbox 内）打，重复多次
python3 - <<'PY'
import urllib.request, json, time
URL = "http://8080-<OBSERVE_SBX>.supone.top/chat/completions"  # 替换 sandbox id
def call(kb):
    body = json.dumps({"messages":[{"role":"system","content":"x"*(kb*1024)},
                                    {"role":"user","content":"hi"}],
                       "max_tokens":1}).encode()
    req = urllib.request.Request(URL, data=body, method="POST",
            headers={"Content-Type":"application/json","Authorization":"Bearer <KEY>"})
    t=time.time()
    try:
        r=urllib.request.urlopen(req, timeout=60); return (r.status, round(time.time()-t,2))
    except urllib.error.HTTPError as e: return (e.code, round(time.time()-t,2))
for kb in (1,16,32,48,64):
    print(kb,"KB:", [call(kb) for _ in range(8)])
PY
# 预期：1KB 全 200；16KB 概率性失败；32KB+ 全 502；64KB+ 全 431

# B) 对照（绕过流量入口）：在目标 sandbox 内对 127.0.0.1:PORT 发同样请求
#    预期：16KB–500KB 全部成功
```

---

## 5. 建议 e2b 侧检查项

`502/431 + 概率性 + 瞬拒（<0.5s）+ 失败请求未到达 upstream` 这组特征，指向流量入口（nginx / envd traffic proxy）对**客户端请求头/体缓冲**的限制或竞态：

1. **nginx 缓冲相关**（若入口是 nginx 类反代）：
   - `client_max_body_size`（过小会 `413`，本例是 `431/502`，更像 header/buffer）；
   - `client_header_buffer_size` / `large_client_header_buffers`（不足触发 `431 Request Header Fields Too Large`）；
   - `proxy_buffer_size` / `proxy_buffers` / `proxy_busy_buffers_size`（转发到 upstream 时 buffer 不足易 `502`）。
2. **为何「概率性」**（16KB 时 1/8 成功）：是否有多入口实例/多 worker 且配置不一致？或连接复用/缓冲在并发下的竞态？请重点排查这一点——确定性阈值无法解释概率性成功。
3. **不同大小落不同错误码**（中段 `502`、更大 `431`）：是否多层 buffer 边界（先撞 proxy_buffer → 502，再撞 header_buffer → 431）？
4. 与公网 e2b（阿里云）流量入口对齐：同样的跨 sandbox 大 body 请求在公网 e2b 是否正常？

---

## 6. claw 侧已隔离 / 已排除

- 非业务死锁：trace 显示 turn 是「9 次 502 后失败退出」，非进程卡死。
- 非 claude-tap：实验 B（loopback）证明 tap 可稳处理 ≥500KB body。
- 非 observe 生命周期：observe `healthz=200`、cluster 一致，全程在线。
- 非上游 LLM：失败请求 <0.5s 瞬拒，从未到达上游。

claw 侧采用「统一 observe + worker 经流量入口访问远程 tap」的架构（目的是 worker 不持有 LLM 凭据）。该架构把 worker→tap 从同 sandbox 的 loopback 改为**跨 sandbox 经流量入口**，从而暴露了本问题。修通 F15 后，claw 将复跑端到端 solve 验证。

---

## 7. 参考

- F14（同一流量入口的路由问题）：`docs/ovs-chat/E2B-TRAFFIC-ROUTING-F14.md`
- observe / tap 设计：`docs/ovs-chat/FC-TAP-SINGLETON-DESIGN.md`
- LLM 使用层与凭据契约：`docs/llm-usage-layer.md`

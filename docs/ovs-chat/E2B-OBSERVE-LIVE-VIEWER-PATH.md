# e2b Observe Live Viewer — 已 superseded by Host 域名方案

Author: kejiqing  
Date: 2026-06-21 (updated 2026-06-22)  
Status: **SUPERSEDED** — 使用 `supone.top` Host 流量，不再走 `/e2b/` path  
Claw 集成方：claw-code（Gateway + FC Observe singleton + worker 内嵌 tap）  
Related: e2bserver Affine「Traffic 访问说明」(`page-29c697b4`)、[FC-TAP-SINGLETON-DESIGN.md](./FC-TAP-SINGLETON-DESIGN.md)

---

## 1. 一句话

**2026-06-22 起**：浏览器 URL 统一为 E2B Host 域名  
`http://{port}-{sandboxId}.supone.top/?session={sessionId}`  
**不再**使用 `http://10.8.0.9/e2b/{port}/{sandboxId}/` path 方案。

Host 方案下 claude-tap Live 在沙箱根路径服务，`fetch('/api/…')` 天然正确，**无需** `CLAUDE_TAP_LIVE_PREFIX_PATH`。

---

## 2. 历史（F15，path 方案，已废弃）

| 项 | 值 |
|----|-----|
| 旧 URL | `http://10.8.0.9/e2b/3000/sbx_*/?session=…` |
| 根因 | path 子路径下 viewer JS `fetch('/api/…')` 打到 nginx 根路径 404 |
| 旧修复 | claude-tap v0.0.10 + `CLAUDE_TAP_LIVE_PREFIX_PATH=/e2b/…` |

---

## 3. 当前 Claw 配置

```bash
CLAW_FC_DOMAIN=supone.top
# 不要设 CLAW_FC_TRAFFIC_PUBLIC_BASE（path 方案已删除）
```

Gateway 返回的 `ovsFolderUrl` / `liveBaseUrl` 示例：

```text
http://3000-sbx_530fe6c53ade.supone.top/ovs?folder=%2Fclaw_ws%2Fproj_3%2Fhome
http://3000-sbx_530fe6c53ade.supone.top/?session={sessionId}
```

---

## 4. 验收

```bash
OBS=sbx_e28cc813aba8
SID=f461373dbfd74348a28deee52c3e6dbe
BASE="http://3000-${OBS}.supone.top"

curl -fsS "${BASE}/api/sessions/traces?session=${SID}" | head -c 200

# 浏览器 ${BASE}/?session=${SID}
# Network：/api/sessions/traces 请求 Host 必须为 3000-${OBS}.supone.top
```

---

## 5. 记录表

见 [FC-OVS-E2E-FAILURES.md](./FC-OVS-E2E-FAILURES.md) **F15**（path 方案 FIXED → Host 方案 superseded）。

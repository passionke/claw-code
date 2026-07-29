# e2b 流量入口：非 ASCII 请求体 PUT → nginx 502（nas-api / 附件上传）

Author: kejiqing  
Date: 2026-07-29  
Claw 集成方：claw-code（Gateway `claw-nas-api` singleton + Admin 会话附件上传）  
关联历史：[`E2B-TRAFFIC-ROUTING-F14.md`](./E2B-TRAFFIC-ROUTING-F14.md)（Host 未进 sandbox）；本单是 **已进对端口 Host，但带二进制 body 的 PUT 被流量 nginx 打成 502**。

---

## 1. 一句话

`claw-nas-api` 在 sandbox 内 `:8090` 正常；经 e2b 流量入口  
`http://8090-{sandboxId}.{domain}/v1/files/...` **PUT** 时：

- **请求体仅为 ASCII** → **200**，文件写入成功  
- **请求体含任意 ≥0x80 字节**（哪怕 **1 字节 `0xFF`**）→ **HTTP 502**，响应体是 **nginx/1.27.5 默认 HTML**，**不是** nas-api 的 JSON

结果：Gateway Admin 上传 JPEG/PNG 等附件必挂（`NAS put_file failed: … 502 Bad Gateway`）。

---

## 2. 调用链（Claw 侧）

```text
Admin / Gateway
  POST /v1/sessions/{id}/files  (multipart)
    → http-gateway-rs session_upload
      → E2bNasApiSingleton.put_file
        → PUT http://8090-{nasApiSandboxId}.{domain}/v1/files/{relPath}
             Content-Type: application/octet-stream
             body: 原始文件字节
          ↑
          此处被 nginx 502（二进制 body）
```

- 上游服务：e2b singleton `claw-nas-api`（Python `ThreadingHTTPServer`，监听 `0.0.0.0:8090`，根目录 `/claw_ws`）  
- 代码：`deploy/e2b/claw-nas-api/server.py`（`do_PUT` `/v1/files/`）  
- 客户端：`rust/crates/http-gateway-rs/src/pool/interactive_backend/e2b_nas_api_singleton.rs`  
- **nas-api 自身不会返回 HTML**；502 HTML = **流量入口 nginx**，请求未以合法方式落到（或未正确转发完）upstream。

旁证：同一 sandbox 用 **e2b envd `files.write`** 写入同路径 JPEG 后，`GET` 同一 URL 可读回完整字节 → **应用与 NAS 写盘正常，问题在「经 nginx 的 PUT + 二进制 body」**。

---

## 3. 复现（给 e2b，可独立于 Claw）

前提：任意在线 sandbox，端口 **8090** 上有 HTTP 服务即可（本报告用 `claw-nas-api`）。

```bash
# 替换为当前 nas-api 流量 URL（PG e2bNasApi.baseUrl 或 8090-{sid}.{domain}）
NAS=http://8090-sbx_XXXXXXXX.ailab.spone.xyz

# ✅ ASCII body → 期望 200
curl -sS -D - -o /tmp/ascii.out \
  -X PUT --data-binary 'ascii-ok' \
  -H 'Content-Type: application/octet-stream' \
  "$NAS/v1/files/local-dev/_diag/e2b_ascii.txt"
# HTTP/1.1 200 …  body: {"written":"..."}

# ❌ 单字节 0xFF → 实际 502 + nginx HTML
printf '\xff' | curl -sS -D - -o /tmp/ff.out \
  -X PUT --data-binary @- \
  -H 'Content-Type: application/octet-stream' \
  "$NAS/v1/files/local-dev/_diag/e2b_ff.bin"
# HTTP/1.1 502 Bad Gateway
# Server: nginx/1.27.5
# body: <html>…502 Bad Gateway…nginx/1.27.5…

# ❌ JPEG magic / PNG magic → 同样 502
printf '\xff\xd8' | curl -sS -o /tmp/jpg.out -w '%{http_code}\n' -X PUT --data-binary @- \
  -H 'Content-Type: application/octet-stream' \
  "$NAS/v1/files/local-dev/_diag/e2b_ffd8.bin"

# ✅ PDF 文件头是可打印 ASCII → 200（说明不是「任意 PUT」坏，而是「非 ASCII 字节」）
curl -sS -o /tmp/pdf.out -w '%{http_code}\n' -X PUT --data-binary '%PDF-1.4' \
  -H 'Content-Type: application/octet-stream' \
  "$NAS/v1/files/local-dev/_diag/e2b_pdf.bin"
```

### 2026-07-29 实测矩阵（local-dev，可复核）

| 请求 | body | 结果 |
|------|------|------|
| `GET /healthz` | — | **200** `{"ok":true,"nasRoot":"/claw_ws"}` |
| `PUT …/e2b_ascii.txt` | `ascii-ok` | **200** |
| `PUT …/e2b_ff.bin` | `0xFF`（1 byte） | **502** `nginx/1.27.5` |
| `PUT …/e2b_ffd8.bin` | `FF D8` | **502** |
| `PUT …/e2b_png.bin` | `89 50 4E 47 …` | **502** |
| `PUT …/e2b_pdf.bin` | `%PDF-1.4` | **200** |
| `GET` 已写入的 ascii 文件 | — | **200** |

当时流量 Host / DNS：

| 项 | 值 |
|----|-----|
| baseUrl | `http://8090-sbx_5f8fbbf1cf23.ailab.spone.xyz` |
| DNS | `8090-sbx_5f8fbbf1cf23.ailab.spone.xyz` → `10.22.28.94` |
| 直打 `http://10.22.28.94/...` + Host | 同样 **502**（仍经流量 nginx） |
| `http://10.22.28.94:8090/...` | Connection refused（8090 未在 ingress IP 上暴露，符合「只走 Host 反代」） |

**跨域名：** `*.ailab.spone.xyz` 与 `*.spone.xyz`（pre-claw-01）上同一现象（ASCII OK / `0xFF` 502）→ **平台流量入口共性，非单一 sandbox / 非单一集群。**

响应头摘录（失败）：

```http
HTTP/1.1 502 Bad Gateway
Server: nginx/1.27.5
Content-Type: text/html
Content-Length: 157
Connection: keep-alive
```

延迟约 **30–50ms**（不像慢超时，更像反代立刻失败或读 body/转上游异常）。

---

## 4. 已排除（Claw / 业务侧）

| 假设 | 结论 |
|------|------|
| nas-api 进程挂了 | 否：同刻 `GET /healthz` 200；ASCII PUT 200 |
| 沙箱未 running | 否：e2b API `state=running`；Gateway `e2bNasApi.online/healthy=true` |
| 文件过大 | 否：1 byte `0xFF` 即 502；同体积纯 `A`×255963 曾 **200** |
| 路径 / 扩展名 | 否：`.bin` / `.jpg` / 无扩展名，二进制皆 502 |
| Content-Type | 否：一律 `application/octet-stream`，ASCII 同 header 却成功 |
| Gateway 客户端 bug | 否：宿主机 `curl` / `urllib`、Gateway 容器内直打 nas URL，行为一致 |
| 仅 local-dev | 否：pre（`spone.xyz`）同样复现 |

---

## 5. 期望行为（验收标准）

对任意 `8090-{sandboxId}.{domain}`（及同等 traffic Host 规则）：

1. `PUT` + `Content-Length` + **任意二进制 body**（含 `0x00`–`0xFF`）原样转到 sandbox 对应端口。  
2. 最小验收：

```bash
printf '\xff' | curl -sf -X PUT --data-binary @- \
  -H 'Content-Type: application/octet-stream' \
  "$NAS/v1/files/<any-writable-rel>/one_ff.bin"
# 期望：HTTP 2xx，且后续 GET 读回单字节 0xFF

# 以及一张真实 JPEG（≥100KB）PUT 成功并可 GET 校验 hash
```

3. 失败时若仍 502，响应/日志应能区分：**upstream 无连接** vs **body 转发损坏**（当前只有通用 nginx 502 HTML，无法区分）。

---

## 6. 请 e2b 侧优先排查

1. **traffic nginx**（版本 **1.27.5**）对 `{port}-{sandboxId}.{domain}` 的 `proxy_pass`：  
   - `proxy_request_buffering` / `client_body_*`  
   - 是否把 body 当文本/UTF-8 处理  
   - 是否有 WAF / `modsecurity` / lua 对高位字节或「文件魔数」拦截（表现像拦截，但返回的是 **502** 而非 403）  
2. 从 ingress 到 MicroVM **8090** 的二层通道：二进制帧是否被截断/重置（对比 ASCII PUT 与 `0xFF` PUT 的 upstream access/error log）。  
3. 用 **sandbox 内** `tcpdump`/`nc` 对比：外网 PUT `0xFF` 时，guest `:8090` **是否收到完整请求**（若收不到 → 纯 ingress；若收到但进程崩 → 再查 guest，当前证据更偏向 ingress）。

---

## 7. Claw 业务影响

- Admin「选文件对话」上传图片/扫描件 → Gateway 报 `NAS put_file failed: nas-api put_file HTTP 502`。  
- 临时绕过（**不作为正式契约**）：e2b envd `Sandbox.files.write` 写入 `/claw_ws/...` 后再 `solve_async`（已验证 JPEG 可读、视觉模型可答）。正式路径仍必须是 **HTTP PUT 经 traffic 域名**，与 SDK Host 约定一致。

---

## 8. 联系 / 取证环境（写报告当日）

| 项 | 值 |
|----|-----|
| Claw Gateway | 本机 `claw-gateway-rs` `:18088`，`clusterId=local-dev` |
| e2b API | `http://ailab.spone.xyz:3300`（见部署 `.env` `CLAW_E2B_*`） |
| 流量域名 | `*.ailab.spone.xyz`（另：`*.spone.xyz` 同病） |
| 流量 VIP（DNS） | `10.22.28.94` |
| 复现服务 | `claw-nas-api` template，guest `:8090` |

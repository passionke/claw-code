# Workbox NeuroGate Admin HTTPS

Author: kejiqing

## Domain

| 用途 | 域名 | 后端 |
|------|------|------|
| **Admin UI（playground）** | `https://neurogate.workbox.spone.xyz` | vw-nginx → `:18765` |
| Gateway API | `https://gateway.workbox.spone.xyz` | vw-nginx → `:18088` |
| e2b Panel | `https://e2b.workbox.spone.xyz` | vw-nginx → `:3000` |
| e2b sandbox traffic | `*.{port}-sbx_*.workbox.spone.xyz` | vw-nginx → e2b traffic `:3001` |

**拼写**：产品名为 **NeuroGate**，Admin 入口域名为 **`neurogate.workbox.spone.xyz`**（含 `u`）。勿使用 `nerogate`（缺 `u` 的误拼）。

常用入口：

- Admin 首页：`https://neurogate.workbox.spone.xyz/admin`
- 项目配置：`https://neurogate.workbox.spone.xyz/admin/project-config`

## nginx（workbox 宿主机）

配置文件：`~/work/myPassword/nginx/conf.d/workbox-neurogate.conf`（vw-nginx 挂载 `conf.d/`）。

```nginx
# neurogate.workbox.spone.xyz → NeuroGate playground / Admin UI :18765
server_name neurogate.workbox.spone.xyz;
```

TLS：Cloudflare DNS-01（lego），证书 `workbox.spone.xyz` wildcard，挂载到 vw-nginx `/etc/nginx/certs/`。

变更后：

```bash
docker exec vw-nginx nginx -t && docker exec vw-nginx nginx -s reload
```

## Gateway `.env` 锚点

```bash
CLAW_CLUSTER_ID=workbox-20260828
CLAW_E2B_DOMAIN=workbox.spone.xyz
CLAW_E2B_API_URL=http://10.8.0.19:3000
CLAW_E2B_SANDBOX_URL=http://10.8.0.19:3002
CLAW_GATEWAY_PUBLIC_HOST=neurogate.workbox.spone.xyz
GATEWAY_HOST_PORT=18088
GATEWAY_PLAYGROUND_HOST_PORT=18765
PLAYGROUND_PUBLIC_GATEWAY_BASE=http://127.0.0.1:18088
```

`PLAYGROUND_PUBLIC_GATEWAY_BASE` 保持 loopback：Admin 容器内 `__proxy__` 反代本机 gateway，不经公网域名。

## 验收

```bash
curl -sS -o /dev/null -w '%{http_code}\n' https://neurogate.workbox.spone.xyz/admin
curl -sS -o /dev/null -w '%{http_code}\n' https://gateway.workbox.spone.xyz/healthz
dig +short neurogate.workbox.spone.xyz   # → 10.8.0.19
```

AFFiNE 运维页：`claw-code` → `docs/ovs-chat/deploy/workbox-neurogate-stack.md`（与本文同步）。

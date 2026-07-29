# 会话附件 OSS 持久化

Author: kejiqing  
Date: 2026-07-29

## 一句话

上传附件时：**NAS** 仍供 worker solve 读盘（短周期可清）；**OSS** 双写长周期对象；DB 记原始 `ossUrl`/`ossKey`；Admin/Playground 历史回放用 Gateway **临时加签** URL。

## 配置（repo `.env`，重启 gateway）

| 变量 | 默认 / 示例 | 说明 |
|------|-------------|------|
| `CLAW_OSS_ENABLED` | `1` | `0`/`false` 关闭双写 |
| `CLAW_OSS_ENDPOINT` | `https://oss-ap-southeast-1.aliyuncs.com` | 区域 endpoint |
| `CLAW_OSS_REGION` | `ap-southeast-1` | V4 签名 region |
| `CLAW_OSS_BUCKET` | `clawcode-sessions` | 桶名 |
| `CLAW_OSS_ACCESS_KEY_ID` / `CLAW_OSS_ACCESS_KEY_SECRET` | — | 读写该桶 |
| `CLAW_OSS_KEY_PREFIX` | `sessions` | 对象前缀 |
| `CLAW_OSS_OBJECT_TTL_DAYS` | `730` | 写入 `ossRetainUntilMs`；lifecycle 天数 SoT |
| `CLAW_OSS_SIGNED_URL_TTL_SECS` | `3600` | 历史/上传响应临时 GET 加签有效期 |

Admin 只读：`GET /v1/gateway/global-settings` → `oss`（不含 SK）；页面「全局配置 → OSS 附件存储」。

## 对象布局

```text
{prefix}/{clusterId}/proj_{projId}/{sessionId}/{uuid8}_{safeName}
```

原始 URL（DB 存）：

```text
https://{bucket}.oss-{region}.aliyuncs.com/{key}
```

## 数据流

1. `POST /v1/sessions/{id}/files` → NAS `put_file` +（启用时）OSS `put_object`
2. `solve_async` 的 `attachments` 整段写入 `gateway_turns.entry_params_json`（含 `ossKey`/`ossUrl`/`ossRetainUntilMs`）
3. `GET .../turns` 对有 `ossKey` 的项补 `ossSignedUrl`
4. UI 用 `ossSignedUrl` 展示图片；旧轮次无 OSS 字段 →「不可预览」

## Lifecycle（运维一次性）

在阿里云控制台为 `clawcode-sessions` 配置生命周期规则：

- 前缀：`sessions/`
- 过期天数：与 `CLAW_OSS_OBJECT_TTL_DAYS`（默认 **730**）一致

Gateway **不**删除 OSS 对象；`ossRetainUntilMs` 仅供 UI/审计判断。

## 签名实现备注

OSS V4（`OSS4-HMAC-SHA256`）：

- **Authorization header**（PUT）：CanonicalHeaders **不含 host**，含 `x-oss-content-sha256` + `x-oss-date`（及存在时的 `content-type`）
- **预签名 URL**（GET）：query 带 `x-oss-additional-headers=host`，CanonicalHeaders 含 `host:{bucket}.oss-{region}.aliyuncs.com`

代码：`rust/crates/http-gateway-rs/src/oss_object_store.rs`。

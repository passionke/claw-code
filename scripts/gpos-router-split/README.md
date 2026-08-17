# GPOS Router 拆分 Runbook

Author: kejiqing

## 本机验收（推荐）

**不碰预发。** 见 [`README-local-test.md`](README-local-test.md)：`apply_local.sh` + `acceptance_smoke.py`。

## 预发拆分（仅运维窗口）

预发从单 project **271** 拆为 **router + kb-qa + ops-analysis（271 纯问数）**。

## 1. 新建 project

| project | role | 说明 |
|---------|------|------|
| gpos-router | `router` | Admin `PUT /v1/projects/{id}/role` body `{ "projectRole": "router" }` 触发 seed |
| gpos-kb-qa | `normal` | 仅 `product-manual-qa` + KB；无 SQLBot |
| 271 | `normal` | 仅问数 skills + SQLBot；移除手册 KB / product-manual-qa |

## 2. delegate-targets

```bash
# router → kb + ops（projId 按预发实际填写）
curl -X PUT "$GW/v1/projects/$ROUTER_PROJ/delegate-targets" \
  -H "Authorization: Bearer $CLAW_ADMIN_TOKEN" \
  -H 'Content-Type: application/json' \
  -d "{\"targets\":[{\"targetProjId\":$KB_PROJ,\"label\":\"kb-qa\",\"capabilityHint\":\"product manual\"},{\"targetProjId\":271,\"label\":\"ops-analysis\",\"capabilityHint\":\"business analytics\"}]}"
```

## 3. ops 嵌套 delegate（场景 7，可选 marketing）

```bash
# 271 启用 delegate_project tool
# Admin draft allowedTools 增加 delegate_project 后 activate，或调用 seed 后手工 merge

curl -X PUT "$GW/v1/projects/271/delegate-targets" \
  -H "Authorization: Bearer $CLAW_ADMIN_TOKEN" \
  -H 'Content-Type: application/json' \
  -d "{\"targets\":[{\"targetProjId\":$MARKETING_PROJ,\"label\":\"marketing\"}]}"
```

## 4. BFF / 评测

- 用户入口 projId 改为 **router**
- 回归见 [`docs/gpos-intent-routing-regress.md`](../docs/gpos-intent-routing-regress.md)

## 5. 验收

见 [`docs/specialist-router-acceptance.md`](../docs/specialist-router-acceptance.md) 场景 1–7。

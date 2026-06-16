# Conversation QA

从线上 Gateway 拉取整通会话（turns / tools / timeline），并生成质量诊断报告。

**Author:** kejiqing

## 环境

| 地址 | 环境 | 用途 |
|------|------|------|
| `http://10.200.2.171:18088` | **生产** | 拉 session 诊断（**ONLY_READ**） |
| `http://192.168.9.252:18088` | **预发** | 写 skill/config、验收 solve |
| `http://10.22.11.19:18088` | **本地** | 本机 dev gateway |

```bash
export GATEWAY=http://10.200.2.171:18088   # 生产 ONLY_READ（与 alfred admin/chat 同后端）
export PROJ_ID=10
```

`fetch_session.py` / `diagnose_session.py` **仅 GET**，禁止对生产地址 POST skill、config、术语等写操作。

## 拉取单会话

```bash
python3 scripts/conversation-qa/fetch_session.py \
  --session-id 9e918b22bd454e2cbdf01fb5956042d6
```

输出目录：`scripts/conversation-qa/cases/<session_id>/session.json`

## 质量诊断

```bash
python3 scripts/conversation-qa/diagnose_session.py \
  --session-id 9e918b22bd454e2cbdf01fb5956042d6
```

生成：`cases/<session_id>/diagnosis.md`

## 目录约定

```
scripts/conversation-qa/
  fetch_session.py      # 拉取 API 原始数据
  diagnose_session.py   # 规则化质量诊断（可迭代）
  cases/                # gitignore，本地缓存
    <session_id>/
      session.json
      diagnosis.md
```

## 数据来源（证据链）

- `GET /v1/sessions/{id}/turns?proj_id=`
- `GET /v1/sessions/{id}/turns/{turn_id}/tools?proj_id=`
- `GET /v1/sessions/{id}/turns/{turn_id}/timeline?proj_id=`（可选）

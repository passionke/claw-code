# FC Worker 上下文架构 — 契约

Author: kejiqing

Status: **实施中**（2026-06-29）

相关：[`fc-nas-workspace.md`](../fc-nas-workspace.md)、[`OVS-INTERACTIVE-SESSION-ID.md`](../ovs-chat/OVS-INTERACTIVE-SESSION-ID.md)

---

## 不变量

1. **`sessionId`**（含 OVS `record_session_id`）= 用户可见会话分桶 + NAS `sessions/{segment}/` 主键
2. **`workerId`** = 执行资源租约，**不是**上下文 key
3. **`home/`** = 项目稳定资料（ds_home），**仅管理后台 materialize 可写**；OVS / resolve worker **只读**
4. transcript / turn 产物在 `sessions/{sessionId}/.claw/` 下（按类型分 jsonl 文件）
5. NAS 逻辑路径在 `{clusterId}/` 之下（`clusterId` = `CLAW_CLUSTER_ID`）

---

## NAS 布局

```text
<export-root>/
  {clusterId}/
    proj_{projId}/
      home/                        # ds_home：管理后台唯一写入口
      sessions/
        {sessionId}/               # 真实目录（禁止 symlink → worker）
          .claw/
            interactive-session.jsonl      # OVS
            gateway-solve-session.jsonl    # resolve
            turns/{turnId}/...
          work/
          ds -> ../../home           # 只读引用
      workers/
        {workerId}/                  # 执行缓存 only
```

---

## Guest mount

| relPath | mountDir | 权限 |
|---------|----------|------|
| `{clusterId}/proj_N/home` | `/claw_ds` | **只读** |
| `{clusterId}/proj_N/sessions` | `/claw_sessions` | 读写 |
| `{clusterId}/proj_N/workers/{workerId}` | `/claw_host_root` | 读写（缓存） |

solve / OVS exec：`cd /claw_sessions/{segment}`，`HOME` 同路径；`CLAW_PROJECT_CONFIG_ROOT=/claw_ds`。

---

## Session 类型

| 类型 | pool_id | transcript |
|------|---------|------------|
| Resolve | `fc-cloud` | `.claw/gateway-solve-session.jsonl` |
| OVS interactive | `fc-interactive` | `.claw/interactive-session.jsonl` |

---

## 禁止路径

- `sessions/{segment}` → `../workers/{workerId}` symlink
- solve `cd /claw_host_root` + worker 根单一 jsonl

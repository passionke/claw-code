# PG session advisory lock × solve cancel 泄漏

Author: kejiqing  
Date: 2026-09-08  
Audience: gateway / 预发运维；统一 review 用  
Status: **已合入修复分支** — `acquire` 后即 `close_on_drop`；单测钉洞 2 + 并发 abort storm；发版后须 252 cancel 风暴再验

---

## 摘要

预发 `192.168.9.252`（`CLAW_CLUSTER_ID=pre-claw-01`）上，并发 `solve_async` 后立刻 cancel，仍可把 **`pre-claw-01:nas-api-singleton`** 的 session advisory lock 泄漏进 sqlx 连接池：持锁连接变 `idle | ClientRead`，被拿去跑无关 SQL，后续 ensure / solve 整集群卡死。

`release-v1.8.14`（PR #156）只盖住了 **「Guard 已持有连接时 abort」**；**锁到手后、Guard 建立前**，以及 **`unlock()` 里 `take()` 之后、unlock SQL 完成前** 被 cancel，仍会带锁回池。

---

## 背景时间线（本轮对话）

| 时间点 | 事实 |
|--------|------|
| 调查 | 任务极慢：几乎全耗在 `ensure_e2b_runtime` 等 `nas-api-singleton` advisory lock；非 LLM |
| 根因 | cancel/abort 后 session lock 留在 pooled connection 上 |
| PR #156 / `release-v1.8.14` | `PgSessionAdvisoryLockGuard`：Drop 时 `close_on_drop`；单测 abort 在 critical section 内 |
| 252 标准升级 | tag → image CI → `up --release` → `e2b-worker-deploy` → `restart`；runtime=`release-v1.8.14` |
| 验证 | 12 路并发 solve + 立刻 cancel → **仍泄漏**；probe solve 180s 仍 `running` |
| 恢复 | `gateway.sh restart`；`nas-api-singleton` advisory 持有数归 0 |

---

## 这把锁是什么意思

不是业务表行锁。

是 PostgreSQL **session advisory lock**：旗挂在**某一根数据库连接**上。

| 锁 key（例） | 含义 |
|--------------|------|
| `pre-claw-01:nas-api-singleton` | 集群同一时刻只许一个任务 ensure nas-api 沙箱 |
| `pre-claw-01:observe-singleton` | 同上，全局 observe |
| `pre-claw-01:observe-proj:98` | 项目 98 的 observe |
| `pre-claw-01:98:0` | 项目 98 worker 槽位 0（重建/轮转） |

**目的：** 多 gateway / 多 solve 不要同时建、探、写同一个 e2b 单例（或同一 worker slot）。

**致命点：** 锁跟连接走。连接回池、被复用跑别的 SQL，锁还在 → 所有人堵在 `pg_advisory_lock`。

实现入口：

- `GatewaySessionDb::with_pg_advisory_lock`（`session_db.rs`）
- 包装：`with_e2b_singleton_role_lock` / `with_e2b_project_observe_lock` / `with_project_e2b_worker_slot_lock`
- key 形如：`{cluster_id}:{role}`、`{cluster_id}:observe-proj:{proj}`、`{cluster_id}:{proj}:{slot}`

---

## 一次 solve：锁在整条流程哪里出现

```
客户端 POST /v1/solve_async
        │
        ▼
gateway 起 tokio 任务（AbortHandle；/v1/tasks/{id}/cancel → abort 整段）
        │
        ▼
① ensure_e2b_runtime_for_proj          ← 卡死主要发生在这里
   │
   ├─ 锁 A: {cluster}:nas-api-singleton
   │     ensure nas-api（探活 / 必要时重建，可能数秒～数十秒）
   │  放锁 A
   │
   └─ 锁 B: {cluster}:observe-proj:{id}  或  {cluster}:observe-singleton
         ensure observe
      放锁 B
        │
        ▼
② worker / LLM …
   （需要时还有锁 C: worker slot，同一套 with_pg_advisory_lock）
```

调用链（节选）：

- `solve_pool` → `ensure_e2b_runtime_for_proj`
- → `ensure_e2b_singleton(NasApi)` → `with_e2b_singleton_role_lock("nas-api-singleton", …)`
- → `ensure_project_observe` → `with_e2b_project_observe_lock`  
  或全局 `ensure_e2b_singleton(Observe)`

相关文件：

- `rust/crates/http-gateway-rs/src/gateway_e2b_singleton_lifecycle.rs`（`ensure_e2b_runtime_for_proj`）
- `rust/crates/http-gateway-rs/src/gateway_project_observe.rs`
- `rust/crates/http-gateway-rs/src/session_db.rs`（锁原语 + Guard）
- `rust/crates/http-gateway-rs/src/routes/fragments/solve.rs` / tasks cancel（abort）

---

## 单把锁内部：从哪开始、到哪释放（6 步）

当前 `with_pg_advisory_lock`：

```
步骤1  pool.acquire()           → 拿一根 conn
步骤2  pg_advisory_lock(key)    →【锁从这里开始】（别人持锁则一直等）
步骤3  PgSessionAdvisoryLockGuard::new(conn)
步骤4  f()                      → 真正的 ensure（探 e2b / 建沙箱…）
步骤5  guard.unlock()
         take(conn) → pg_advisory_unlock(key)  →【正常路径锁在这里结束】
步骤6  drop(conn) → 回连接池
```

正常成功：`1 → 2 拿锁 → 3 → 4 → 5 放锁 → 6 回池` ✅

---

## 每种情况怎样（统一对照）

「cancel」= `POST /v1/tasks/{taskId}/cancel` → `AbortHandle.abort()` → future 被丢弃。

| 情况 | 卡在哪 | 锁在谁身上 | 当前行为 |
|------|--------|------------|----------|
| 正常跑完 | 1→6 | 步骤 2～5 | 5 放锁，6 回池 ✅ |
| ensure 返回 Err | 步骤 4 | 仍在 Guard | 仍走 5 unlock ✅ |
| **cancel 在步骤 4**（干活中） | Guard 握 conn | Drop → `close_on_drop` | 连接断 → PG 清 session 锁 ✅ **#156 / v1.8.14 修的** |
| cancel 在步骤 2 等待中 | 尚未持锁 | 通常无锁 | 一般安全；极端竞态另论 |
| **cancel 在步骤 2 刚成功、步骤 3 前** | 已持锁，无 Guard | 裸 conn | **回池锁不放** ❌ 洞 1 |
| **cancel 在步骤 5：take 后、unlock SQL 完成前** | Guard 已空 | 锁仍在 conn | Drop 空操作；**回池锁不放** ❌ 洞 2 |
| 泄漏之后 | 后续卡在步骤 2 | idle 连接仍持锁 | 全员排队；连接可被复用跑无关 SQL |

锁生命示意：

```
        步骤2 成功
            │
            ▼
     ┌──────────── 锁活着 ────────────┐
     │  步骤3 Guard / 步骤4 ensure      │
     │  步骤5 unlock 成功 ──► 锁死     │  ✅
     │  cancel@步骤4 ──► 断连接 ──► 锁死 │  ✅（v1.8.14）
     │  cancel@步骤2后/3前 ──► 回池     │  ❌
     │  cancel@步骤5 take后 ──► 回池    │  ❌
     └────────────────────────────────┘
```

---

## 为什么 v1.8.14「没拦住」

不是没部署，是 **修窄了**。

1. Guard Drop / `close_on_drop` **只在 Guard 还握着 `conn` 时**有效。
2. 单测 `singleton_role_lock_releases_when_holding_task_is_aborted`：先进 critical section（`pending`），再 abort —— **永远在步骤 4**，测不到洞 1 / 洞 2。
3. 并发 cancel：多人堵在步骤 2；有人拿到锁后落在洞里 → 泄漏。洞外路径（步骤 4 abort）仍会打出 Drop warn。

252 验证时：

- 有 warn：`guard dropped … key=pre-claw-01:observe-proj:98` → 步骤 4 路径生效。
- **没有** `nas-api-singleton` 的同等 Drop warn，但该 key 被 idle 连接持有 → 泄漏发生在 Guard 管不到的窗口。

---

## 252 验证证据（2026-09-08）

环境：`deployImageTag=release-v1.8.14`，cluster `pre-claw-01`。

操作：proj 98，12 路并发 `POST /v1/solve_async`，紧接着全部 `POST /v1/tasks/{id}/cancel`（状态均为 `cancelled`）。

结果：

1. `pg_try_advisory_lock(hashtext('pre-claw-01:nas-api-singleton'))` **连续 ~15s 失败**。
2. `pg_locks` 持有者形如：`pid=… | idle | ClientRead | granted=t`，query 已是无关语句（如 `SELECT project_role FROM project_config…`）—— **锁随连接回池并被复用**。
3. 另有多条 `active | advisory | granted=f`，query=`SELECT pg_advisory_lock(hashtext($1::text))` —— 排队。
4. 后续 probe solve：**180s 仍 `status=running`**。
5. 终止持锁 backend 后锁可转移到另一 idle 连接（同类泄漏）；最终 **`gateway.sh restart`** 后该 key advisory count = 0，healthz 恢复。

脚本（本机曾 scp）：`tmp/verify-cancel-lock.sh`（宿主机 `/tmp/verify-cancel-lock.sh`）；PG 经 `admin@192.168.9.250` → `docker exec claw-infra-postgres psql`。

---

## 二次修复（`fix/pg-advisory-lock-cancel-holes`）

已按讨论落地（一条路径）：

1. `with_pg_advisory_lock`：`pool.acquire()` 后立刻 `close_on_drop()`；`pg_advisory_unlock` 仅尽力；drop 关连接。
2. 去掉依赖「Guard 仍握 conn」的 unlock/`Drop` 分叉（洞 2 的根）。
3. 单测盯死：
   - `session_advisory_lock_pool_simulation_return_without_close_leaks` — 记录旧泄漏形态
   - `abort_after_unlock_take_must_not_leak` — 洞 2（曾作破坏信号：无 close 则失败）
   - `session_advisory_lock_concurrent_abort_waiters_must_not_leave_key_held` — 并发模型
   - `singleton_role_lock_releases_when_holding_task_is_aborted` — PG，步骤 4 abort
   - `singleton_role_lock_releases_after_concurrent_abort_storm` — PG，holder+waiters abort storm（需 `CLAW_GATEWAY_TEST_DATABASE_URL`）

发版后须用 252 cancel 风暴脚本再验一次。

---

## 修复意见（供 review，一条路径）

**目标一句话：** 步骤 2 一旦成功，直到「unlock 成功」或「连接关掉」，中间任意 abort 都不得把仍持锁的连接还回池。

推荐（KISS，不新增第二套锁 / 不扫 `pg_locks` 兜底）：

- 在 `with_pg_advisory_lock` 内，**`pg_advisory_lock` 成功后立刻对该 `PoolConnection` `close_on_drop()`**（或从 acquire 起就标记：凡走 session advisory 的连接永不复用）。
- 正常路径仍可 `pg_advisory_unlock`（尽力），然后 drop 关连接。
- 代价：ensure 路径多断 PG 连接；频率可接受。

备选（改动面更大，需单独对齐）：`BEGIN` + `pg_advisory_xact_lock`，事务 Drop/rollback 自动放锁 —— 与现有 session 锁模型二选一，不要并行两套。

**不要做：** 定时扫锁、旁路第二把锁、「泄漏了再修」的运维兜底当产品路径。

### 测试必须补上

| # | 场景 |
|---|------|
| 1 | abort 卡在 `pg_advisory_lock` 等待（并发排队） |
| 2 | abort 落在 lock 成功 ↔ Guard 建立（洞 1） |
| 3 | abort 落在 `unlock` await（洞 2） |
| 4 | 集成：多路 `solve_async` + 立刻 cancel，断言数秒内 `nas-api-singleton` 可 `pg_try_advisory_lock` |

现有「仅 abort critical section」单测保留，但**不能**再当唯一回归。

---

## 运维应急（未修前）

1. 查持锁：`pg_locks` + `pg_stat_activity`，`objid = hashtext('pre-claw-01:nas-api-singleton')`（换本环境 cluster）。
2. 症状：holder `idle` + 无关 query；多人 `wait_event=advisory`。
3. 恢复：`pg_terminate_backend(holder_pid)` 或 **重启 gateway**（清连接池）；仅 kill 一个 pid 可能把锁交给下一个已泄漏连接。

---

## Review 清单

- [ ] 认同「锁含义 + 6 步 + 洞 1/2」描述与代码一致  
- [ ] 认同 v1.8.14 未闭环，需二次修复再发版  
- [ ] 认同「持 session advisory 的连接不回池 / close_on_drop」为默认修复路径（或明确改选 xact_lock）  
- [ ] 认同补测试表 1～4，尤其并发 cancel  
- [ ] 通过后实现 → CI → 新 release tag → 252 用同一 cancel 风暴脚本验收  

---

## 相关文档

- `docs/multi-gateway-cluster.md` — cluster 单例与 advisory 串行（机制概述）
- `docs/e2b-core-lifecycle-governance-plan.md` — ensure / 巡检须持 role lock
- `deploy/docs/pre-252-e2b-pipeline.md` — 252 升级路径

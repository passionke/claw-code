# Chunk / 渲染「时间密集度」怎么定义、怎么算

Author: kejiqing

你说的是 **「很多 trunk 在时间上挤在一起被渲染」**，不是「一共有多少个 chunk」。  
这是 **时间序列上的到达/渲染密度**，必须分层、用同一套时间戳算，不能混用 claude-tap 的 1269 条（那里 **没有 per-chunk 接收时间**）。

---

## 1. 四层时钟（不要混）

| 层 | 时间戳从哪来 | 度量什么 |
|----|--------------|----------|
| **L0 上游** | claude-tap `sse_events[]` 顺序；`created` 只有秒级 | 模型 **生成** 粒度（1～8 字/chunk），**不能**算 1ms 密集度 |
| **L1 网关 SSE** | `biz.report.delta` 的 `serverDeltaMs` / `seq` | 网关 **发出** 事件的密集度 |
| **L2 浏览器收包** | `performance.now()` 或抓包 recv 时刻 | EventSource **收到** 事件的密集度 |
| **L3 屏幕渲染** | `requestAnimationFrame` 回调时刻 / 可见字数采样 | 用户 **看到** 的更新密集度（rAF 会把 L2 再合并） |

**体感「一瞬间蹦很多字」** 通常看 **L3**；**根因在 L0～L1 还是 L2** 要靠 L1/L2 对比。

---

## 2. 基本对象：事件流

一条 turn 的 live 流是一串事件：

\[
E = \{(t_1, b_1), (t_2, b_2), \ldots, (t_N, b_N)\}
\]

- \(t_i\)：该层的时间（毫秒，单调非降）
- \(b_i\)：该条 `textLen`（字节/字符长度）

Admin SSE 里 L1 的 \(t_i\) 用 **`serverDeltaMs`**（相对 `biz.report.start`）。  
浏览器 L2 的 \(t_i\) 用 **`clientDeltaMs = round(performance.now() - t0)`**。

---

## 3. 核心指标（密集度）

### 3.1 桶密度（最直观：「1ms 里几个 chunk」）

取桶宽 \(W\) ms（常用 **1** 或 **16**≈一帧）：

\[
\text{bucket}(t) = \left\lfloor \frac{t}{W} \right\rfloor
\]

\[
D_W = \max_b \#\{i : \text{bucket}(t_i) = b\}
\]

- **`max_bucket_count_1ms`**：同一毫秒内 **最多几条 delta**（L1 或 L2 各算一遍）
- **`max_bucket_count_16ms`**：同一帧时间内最多几条（更接近渲染）

**解读**：`max_bucket_count_1ms = 17` ⇒ 存在某一毫秒收了 17 个事件（L2），或网关在该 ms 发出了 17 条（L1）。

### 3.2 同时到达比例（Simultaneity ratio）

\[
\text{simulRatio} = \frac{\#\{i>1 : t_i = t_{i-1}\}}{N-1}
\]

（\(t\) 取整到 ms 后比较）

- 接近 **1**：几乎每条都和上一条同毫秒 → **极密**
- 接近 **0**：时间分散

### 3.3 到达间隔 IAT（Inter-Arrival Time）

\[
\text{IAT}_i = t_i - t_{i-1} \quad (i \ge 2)
\]

报告：

| 统计量 | 含义 |
|--------|------|
| `iat_median_ms` | 典型间隔 |
| `iat_p95_ms` | 长尾 |
| `iat_min_ms` | 最小间隔（0 表示同桶） |
| `iat_cv` | `std(IAT)/mean(IAT)`，变异系数，越大越「一阵一阵」 |

### 3.4 突发度 Burstiness（峰值 / 均值）

在窗口 \(W\) 上算速率 \(r_b = \#\text{events in bucket } b / W\)。

\[
\text{burstiness} = \frac{\max_b r_b}{\mean_b r_b}
\]

- **≈1**：均匀
- **≫1**：有明显突发（你体感的「卡一下然后一坨」常伴随 burstiness 高 + 单条 `textLen` 大）

### 3.5 载荷密度（字/秒，不是 chunk 数）

\[
\text{charsPerSec} = \frac{\sum_i b_i}{t_N - t_1 + \epsilon}
\]

同样时间内 **chunk 很多但 each 1 字** vs **chunk 少但 each 500 字**，体感不同；必须和 **`max_text_len`** 一起看。

### 3.6 可见更新台阶（L3，用户体感）

每隔 \(\Delta\) ms 采样可见字数 \(L(t)\)：

\[
\text{step}_k = L(t_k) - L(t_{k-1})
\]

- **`max_step_chars`**：单次采样之间最多跳多少字（>200 即「框里突然多一大段」）
- 与 L1 的 `max_text_len` 对照：若 L1 单条就 500 字，L3 必然一次跳 500

---

## 4. 怎么量（本仓库）

### 4.1 网关 L1（推荐，可复现）

```bash
python3 scripts/measure_render_density.py \
  --gateway http://127.0.0.1:18088 \
  --session-id <sid> --turn-id <tid> --ds-id 1 \
  --bucket-ms 1,16 --out /tmp/density.json
```

脚本在 **读 SSE 的当下** 打 `recvMonoMs`，输出：

- L1：`serverDeltaMs` 桶密度、IAT、simulRatio、burstiness
- L2：`recvMonoMs` 桶密度（含 proxy + 内核缓冲）
- 每条 delta 的 `textLen` 分布

### 4.2 浏览器 L2/L3

Console 过滤 `[biz-report-stream]`，或任务结束后：

```javascript
window.__bizReportDeltaLogByTurn?.['T_xxx']  // 每条 { clientDeltaMs, serverDeltaMs, textLen, seq }
window.__bizReportDensityByTurn?.['T_xxx']  // 汇总 max_bucket_count_1ms 等
```

Admin 在 `useBizReportStream` 里对 **每条 delta** 追加 log 并 **在线算密度**（见代码）。

### 4.3 claude-tap 1269 条

只能算 **L0 顺序 + 字长分布**，**不能**算 1ms 密集度（无 per-chunk recv 时间）。  
能算：`chunk_len_median=2`、`N/ duration_ms ≈ 75 chunk/s`（平均），**不能**代替 burst。

---

## 5. 「真流式、不卡顿一大段」建议阈值（可调）

| 指标 | 建议（L1 或 L2） | 坏例 |
|------|------------------|------|
| `max_bucket_count_1ms` | ≤ 8 | 同一 ms 10+ 条 |
| `max_text_len` | ≤ 64（catch-up 48 + 上游 8） | 单条 500+ |
| `max_step_chars`（L3，500ms 采样） | ≤ 80 | 一次跳 500 字 |
| `simulRatio` | ≤ 0.3 | >0.7 几乎全同 ms |
| `large_delta_count`（textLen≥200） | 0 | catch-up 未分片 |

---

## 6. 和当前 bug 的对应关系

| 现象 | 优先看 |
|------|--------|
| 先空很久，再整段出现 | L1 `max_text_len`、首包 catch-up 是否分片 |
| 字在动但「一顿一顿」 | L0 chunk 极碎 + L2 `max_bucket_count_16ms` |
| seq=0 全程 | 没有 L1 事件，**谈不上渲染密集度**（量的是 0 条流） |
| hasReport=true 但无 delta | 协议字段 ≠ 真有 SSE 流 |

---

## 7. 证据链要求

任何「已真流式 / 已修复卡顿」结论，至少贴：

1. `measure_render_density.py` 输出 JSON 片段（含 `max_bucket_count_1ms`、`max_text_len`）  
2. 或 `__bizReportDensityByTurn[turnId]` 截图/复制  
3. 同一 turn 的 `delta_count > 0`

缺则只能标 **待验证**。

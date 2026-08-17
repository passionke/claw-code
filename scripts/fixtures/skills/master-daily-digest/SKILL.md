# master-daily-digest

对配对学徒做**业务日日报**（默认 `bizdate=昨天`，时区以项目 CLAUDE / 调度为准，常见 Asia/Shanghai），并三写交付：
1. **对话回复**：完整中文报告写在本轮最终回答
2. **Mind 文档**：写入 `magic-ai-daily-report`（[文件夹](https://mind.maxiot-inc.com/folders/5387be57-21dc-48ad-91d1-f18f3d059174)）下对应产品子目录
   - IPOS → `IPOS/`（`parentId=179fe2f8-c0cf-494a-b082-a303aad48dce`）
   - GPOS → `GPOS/`（`parentId=dd5fd1dc-c346-4675-9e14-b6b5ca17c1e6`）
   - `docSpaceId=cd4984ca-26c4-4e1f-9f41-c443c88d9ad2`
3. **钉钉群简报**：文档创建后，用加签机器人推送**简报**（群通知仍走钉钉机器人，完整报告不再写钉钉文档）

结构对齐历史深度报告（例：Mind 内 `IPOS 日报简报` / `GPOS 日报简报`）。

## 硬规则
- 读学徒：**必须先**跑 `fetch_digest_inputs.py` + `aggregate_apprentice_day.py`，以产出的 **`stats.md`（及 day/prev/lookback json）为准**；无网才退回 master MCP，且 MCP 取 store 时必须 turns 上的 `extraSession`。
- **`store_id` 在 turns.extraSession**，不在 session 列表。禁止因 session API 无字段写 N/A。
- **门禁**：未读 `stats.md` 的门店复访/耗时段前，禁止写 store/耗时 N/A；`stats.md` 有数则原样引用。
- **需求满足率**与 store_id 无关；耗时用 `finishedAtMs-createdAtMs`。
- **报告章节不得省略**（见「强制大纲」）；可写「本日无」但不可整节删除。
- 客观量级（会话/轮次/store/耗时）**只允许来自 stats.md**；语言/意图/满足率/样例来自对 `day.json` turns 的研判，须与 stats 会话数对得上（样例须能在 day.json 找到）。
- 完整报告只写 **Mind MCP** `import_document`；群里只发简报。凭据只读 env `Webhook` / `security`。
- **文档标题**按产品命名，禁止「学徒 / Master Observer」等字样：
  - IPOS：`IPOS 日报简报 YYYYMMDD`
  - GPOS：`GPOS 日报简报 YYYYMMDD`
  - 汇总：`IPOS 整体汇总 YYYYMMDD-YYYYMMDD` / `GPOS 整体汇总 YYYYMMDD-YYYYMMDD`
- GPOS：空间 10 无业务数据（曾临时用于全时段分析）；标题与对外表述不要写「学徒 10」。
- 不改学徒配置、不开 repair_run（除非用户同时要求）。0 会话也要报告+文档+简报。

## 强制大纲（Mind 正文 + 对话全文）
1. 元信息：D / P / lookback / 时区 / 数据来源（须写明用了 stats.md）
2. **昨日 vs 前日对照表**（数字抄 stats.md）
3. **门店复访**（抄 stats.md，含名单；可截断但须有复访率）
4. **每轮耗时**（抄 stats.md）
5. **语言与意图分析**（必有）
   - **D 日语言分布**：按 userPrompt 书写体系统计轮次或会话（泰/英/中/其他），给数量与占比；总和须覆盖 D 日样本
   - 高频意图类别（3–8 类）
   - 不满/重复提问等信号（可无则写无）
6. **样例会话**（必有）：表格，至少 6 行（不足则全列），列含时间或 sessionId 短码、轮次、语言、store_name（有则填）、prompt 摘要；行须来自 `day.json`
7. 要点与建议（≤5 条）
8. 文末生成时间

## Steps
1. `apprentice_list`；`D=bizdate`，`P=D-1`，lookback `[D-30,D)`。
2. **必跑**：
   ```bash
   export GATEWAY_BASE="${CLAW_GATEWAY_BASE:-http://10.200.2.171:18088}"
   python3 skills/master-daily-digest/scripts/fetch_digest_inputs.py \
     --gateway "$GATEWAY_BASE" --apprentice-id N --bizdate YYYYMMDD \
     --tz Asia/Shanghai --out-dir /tmp/digest_N
   python3 skills/master-daily-digest/scripts/aggregate_apprentice_day.py \
     --bizdate YYYYMMDD --apprentice-id N --in-dir /tmp/digest_N \
     --out /tmp/digest_N/stats.md
   ```
3. 读 `stats.md` + 抽样读 `day.json` turns：满足率 v2、语言分布、意图、样例表。
4. 按**强制大纲**写全文 → Mind MCP 服务器 `mind` 的 `import_document`（`docSpaceId` + 对应产品 `parentId` + 产品标题）→ 得到文档 URL（`https://mind.maxiot-inc.com/docs/{resourceId}`）。
5. `send_dingtalk_brief.py`（数字来自 stats.md；`--doc-url` 用 Mind 链接）。
6. 最终回答 = 全文 + Mind URL + 简报结果。

Author: kejiqing

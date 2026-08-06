# master-cjk-thai-repair

专攻 **泰文场景回答飙中文（CJK 污染）**。每日对配对学徒抽病例 + 对照集，在观察空间做最小 skill/CLAUDE 变体，重放评估；仅当缓解且对照无伤时，`promote_to_apprentice_draft`（永不 activate 学徒），并钉钉提案简报。

与通用 `master-quality-repair` 并存；本 skill 只处理 CJK/泰文污染。

## 硬规则
- 副作用只经 master MCP `claw-master-observer` 状态机；不经其它旁路改学徒生产配置。
- 业务日 D = **Asia/Shanghai 昨天** YYYYMMDD（勿用 UTC 日界）。
- 基线 = 原 turn `reportBody`；变体只 activate 在 **observation**；不先无补丁重放。
- 门禁：`scripts/score_cjk_replay.py` 产出 `promote_recommended=true` 才 promote + 钉钉。
- 钉钉提案用本包 `send_cjk_proposal_dingtalk.py`（脚本内写死 webhook/加签）；与日报 env `Webhook`/`security` 无关。
- 无 CJK 病例：不开 promote、不发提案钉；最终回答说明即可。
- **编排**：耗时/可并行步骤用内置 **Agent** 工具交给 subagent（见下）；主 agent 只握 MCP 状态机与 promote/钉钉。

## Subagent 分工（推荐）
用 `Agent`（可同轮并行多个 Foreground；或 `run_in_background=true` + `AwaitAgent`）：

| 角色 | 谁做 | 产出 |
|------|------|------|
| **extract** | subagent | `/tmp/cjk_N/day.json` + `eval_set.json`（含 `inventoryJson`） |
| **patch + MCP 状态机** | **主 agent** | repair_run / sync / observation 补丁 activate / `observation_replay` |
| **replay-collect** | subagent | 等观察空间重放结束 → `replay.json` + `score.json`/`score.md` |
| **promote + 钉钉** | **主 agent** | 仅 `promote_recommended` 时 |

主 agent **不要**自己长轮询重放会话；交给 replay-collect。extract 与「读 stable 配置草稿思路」可并行，但 `inventory_put` 必须等 extract 完成。

### extract subagent prompt 模板
```
只做数据：GATEWAY_BASE=... apprentice=N bizdate=YYYYMMDD tz=Asia/Shanghai out=/tmp/cjk_N
1) 跑 skills/master-daily-digest/scripts/fetch_digest_inputs.py（或等价 HTTP）得到 day.json
2) 跑 skills/master-cjk-thai-repair/scripts/build_cjk_eval_set.py → eval_set.json
3) 返回：cjkCount/controlCount 与两个文件路径；不要改任何项目配置、不要调 master MCP 写操作
```

### replay-collect subagent prompt 模板
```
只做等待与评分：gateway=... observationProjId=O run 已 observation_replay
1) 把 replay_results_get / run.replay_session_ids 落到 /tmp/cjk_N/replay_sessions.json
2) python3 skills/master-cjk-thai-repair/scripts/wait_and_collect_replay.py \
     --gateway ... --observation-proj-id O \
     --sessions-json /tmp/cjk_N/replay_sessions.json \
     --out /tmp/cjk_N/replay.json
3) python3 skills/master-cjk-thai-repair/scripts/score_cjk_replay.py \
     --eval-set /tmp/cjk_N/eval_set.json --replay-json /tmp/cjk_N/replay.json \
     --out-dir /tmp/cjk_N
4) 返回 score.json 摘要；不要 promote、不要发钉钉
```

## Steps
1. `apprentice_list`；prompt 指定学徒则只处理这些 id。
2. **Agent(extract)**：拉 D 日 + `build_cjk_eval_set.py`（见上）。
3. 若 `cjkCount==0` → 结束（对话说明），不要 `repair_run_open`。
4. 主 agent MCP 序列（不可跳；不可交给会乱序的 subagent）：
   1. `repair_run_open`
   2. `inventory_put`（用 extract 的 `inventoryJson`）
   3. `observation_sync_from_apprentice` → synced
   4. 最小 skill/CLAUDE 补丁（泰文零 CJK）→ `observation_config_put_draft` → commit → activate → patched
   5. `observation_replay`
5. **Agent(replay-collect)**：`wait_and_collect_replay.py` + `score_cjk_replay.py`。
6. 主 agent：`repair_run_analyze`（写入 score 摘要）。
7. 若 `promote_recommended`：
   - `promote_to_apprentice_draft`
   - `send_cjk_proposal_dingtalk.py`
8. 最终回答：规模、指标、是否推草稿、钉钉结果；提醒人工 review 后再 activate。

Author: kejiqing

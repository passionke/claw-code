# GPOS 产品手册问答 · 上线运维手册

Author: kejiqing

面向预发 / 生产的**可执行**运维说明：双语静态 KB、三路意图（闲聊 / 产品手册 / 经营问数）、Admin 同构配置发布、NAS 同步与验收回滚。

## 命名边界

| 名称 | 指什么 | 不指什么 |
|------|--------|----------|
| **GPOS 经营助手** | claw 项目（271/27）整包能力 | QueryX 品牌本身 |
| **QueryX** | Boss 报表 / 经营问数 **BFF 契约**（[`analysis-api-queryx-bff.md`](analysis-api-queryx-bff.md)） | 产品手册 KB、整条助手、claw-code 平台 |
| **手册 KB** | `home/kb` 静态原文 + `product-manual-qa` | 经营数据答案 |

关联文档：

| 文档 | 用途 |
|------|------|
| 本文 | **上线与日常运维真源** |
| [`docs/gpos-intent-routing-regress.md`](gpos-intent-routing-regress.md) | 三路意图回归检查清单 |
| [`docs/gpos-assistant-prompt-content.md`](gpos-assistant-prompt-content.md) | CLAUDE / Rules 粘贴参考 |
| [`scripts/gpos-manual-crawl/`](../scripts/gpos-manual-crawl/) | 爬取工具（产出本地 KB，不入库） |
| [`scripts/gpos-manual-eval/`](../scripts/gpos-manual-eval/) | 冒烟 / Live 评测工具 |
| [`docs/project-config-model.md`](project-config-model.md) | `project_config` / `git_sync` 模型 |
| [`docs/analysis-api-queryx-bff.md`](analysis-api-queryx-bff.md) | QueryX 问数 BFF 契约（仅问数） |

**边界：** 业务知识库（GPOS 手册及后续其它域 KB）**不进 claw-code**。仓库只保留爬取 / 同步 / 评测脚本与 Admin 配置 fixtures；KB 正文落本地缓存或 NAS `home/kb`。

---

## 1. 能力边界（上线前对齐）

```text
用户问题
  ├─ 闲聊 / 能力外     → Skill(self-introduction)     禁止 MCP / 不查 KB
  ├─ GPOS 产品 how-to  → Skill(product-manual-qa)     查 home/kb，禁止 SQLBot
  └─ 经营问数 / 诊断   → 既有分析 skills + SQLBot     禁止用手册当数据答案
```

**语言路由（产品手册）**

| 用户输入 | KB 目录（worker） | 官方链接前缀 |
|----------|-------------------|--------------|
| 含泰文字符 | `/claw_ds/home/kb/th/` | `https://gpos.co.th/th/user-manual/` |
| 其他（中 / 英 / …） | `/claw_ds/home/kb/en/` | `https://gpos.co.th/en/user-manual/` |

- 手册正文：**只爬取原文**，禁止用大模型翻译/改写后再入库。
- 对外协议不变：`SolveRequest`（`projId` + `userPrompt` + `extraSession`）；BFF 仅可换 `projId` 做灰度。

---

## 2. 环境与路径

### 2.1 项目

| 环境 | 网关 | Cluster | 推荐 projId | 说明 |
|------|------|---------|-------------|------|
| 预发 | `http://192.168.9.252:18088` | `pre-claw-01` | **271** | 已验收双语路由 |
| 生产 | 以实际部署为准 | 以实际为准 | **27**（或灰度新 id） | 上线前按本文走完整清单 |

### 2.2 预发磁盘布局（e2b NAS）

| 角色 | 主机 | 路径 |
|------|------|------|
| Gateway | 192.168.9.252 | Docker `claw-gateway-rs`，`CLAW_WORK_ROOT=/var/lib/claw/workspace`（会话多在 volume；**配置物化主落 NAS**） |
| NAS | **192.168.9.250** | `/data/claw-nas/pre-claw-01/proj_<id>/` |

生效配置目录（activate 后会变）：

```bash
ssh admin@192.168.9.250 \
  'readlink -f /data/claw-nas/pre-claw-01/proj_271/home/project_home_def'
# 例：.../home/.claw/project-home-versions/<contentRev>
```

Worker 将该 version 根挂为 `/claw_ds`：

| 逻辑路径 | 物理路径（NAS version 根下） |
|----------|------------------------------|
| `/claw_ds/CLAUDE.md` | `CLAUDE.md` |
| `/claw_ds/.claw/skills/<name>/SKILL.md` | `.claw/skills/...` |
| `/claw_ds/home/kb/{en,th}/` | `home/kb/{en,th}/` |

**重要：** 每次 `activate` 会切到**新的** `contentRev` 目录。KB 必须 **rsync 到当前 `project_home_def` 指向的版本**，否则 worker 读到空 KB。

### 2.3 本地 KB 缓存（不入库）

默认爬取落盘（`.gitignore` 的 `knowledge/`）：

```text
knowledge/gpos-user-manual/   # 或 $GPOS_MANUAL_KB
  index.md
  en/ …          # EN 手册
  th/ …          # TH 手册
  eval/          # 本地跑批产物（勿 rsync 到运行时 kb）
```

仓库内只提交 `scripts/gpos-manual-crawl/` 与 `scripts/gpos-manual-eval/`。未来其它业务 KB 同样：脚本进仓、正文走 NAS / 独立存储。

---

## 3. 上线标准流程（推荐顺序）

> 原则：**配置走 Admin 同构 API（有 `contentRev` 记录）**；**KB 走爬取 + rsync/git_pull**。不要手工改 NAS 上的 CLAUDE/skills 冒充发布。

### Step A — 爬取双语 KB（本机或 CI）

```bash
cd /path/to/claw-code
python3 scripts/gpos-manual-crawl/crawl_gpos_user_manual.py --lang all --delay 0.2
```

检查：

```bash
python3 - <<'PY'
import json
from pathlib import Path
m=json.loads(Path('knowledge/gpos-user-manual/manifest.json').read_text())
print(m)
assert m.get('en_count',0) > 100 and m.get('th_count',0) > 100
assert Path('knowledge/gpos-user-manual/en/membership/add-member-back-office.md').exists()
assert Path('knowledge/gpos-user-manual/th/membership/add-member-back-office.md').exists()
print('kb ok')
PY
```

### Step B — 准备配置内容（仓库 fixtures）

| 文件 | 作用 |
|------|------|
| [`scripts/fixtures/skills/product-manual-qa.SKILL.md`](../scripts/fixtures/skills/product-manual-qa.SKILL.md) | 手册检索 + 语种硬锁 |
| [`scripts/fixtures/skills/self-introduction.SKILL.md`](../scripts/fixtures/skills/self-introduction.SKILL.md) | 闲聊引导（产品 how-to 不走此 skill） |
| [`docs/gpos-assistant-prompt-content.md`](gpos-assistant-prompt-content.md) | CLAUDE / Rules 参考文案 |
| [`scripts/fixtures/proj271_skills_with_product_manual.json`](../scripts/fixtures/proj271_skills_with_product_manual.json) | **整表 skills 备份样例**（合并用） |

### Step C — Admin 同构发布（必须有记录）

网关 Admin MCP：`POST $GATEWAY/v1/admin/mcp`（Bearer Admin Token）。

工具顺序：

1. `project_config_get({projId})` — 拉齐当前 skills / claude / rules  
2. **合并**写入 draft（见下节「整表覆盖陷阱」）  
   - `project_skills_put_draft`  
   - `project_claude_put_draft`  
   - `project_rules_put_draft`  
3. `project_config_commit_draft({projId, note})` → 得到 `savedContentRev`  
4. `project_config_activate({projId, contentRev})` → `materialized: true`

`note` 示例：

```text
feat: product-manual bilingual kb routing th/en (kejiqing)
```

也可用 Gateway Admin UI 完成同等 draft → commit → activate（同样写入 `contentRev` 历史）。

#### 整表覆盖陷阱（必读）

`project_skills_put_draft` / `project_rules_put_draft` 传入的数组会作为 draft **整表**，**不是**按 name merge。

错误做法：只传 `product-manual-qa` 一个 skill → 诊断 skills 全部丢失。  
正确做法：`project_config_get` 取全量 → 替换/追加目标项 → **整表写回**。

### Step D — 同步 KB 到「当前生效」version（activate 之后）

预发示例：

```bash
PROJ=271
CLUSTER=pre-claw-01
NAS=admin@192.168.9.250
VER=$(ssh "$NAS" "readlink -f /data/claw-nas/${CLUSTER}/proj_${PROJ}/home/project_home_def")
echo "sync -> $VER/home/kb"

rsync -az --delete \
  --exclude 'eval/' \
  --exclude 'README.md' \
  knowledge/gpos-user-manual/ \
  "${NAS}:${VER}/home/kb/"

# 抽检双语文件与链接语种
ssh "$NAS" "grep -m1 source_url ${VER}/home/kb/en/membership/add-member-back-office.md; \
            grep -m1 source_url ${VER}/home/kb/th/membership/add-member-back-office.md; \
            test -f ${VER}/.claw/skills/product-manual-qa/SKILL.md && echo skill_ok"
```

期望：

- EN：`source_url: https://gpos.co.th/en/user-manual/...`
- TH：`source_url: https://gpos.co.th/th/user-manual/...`
- skill 目录存在 `product-manual-qa`

本地非 NAS 环境：

```bash
scripts/gpos-manual-crawl/sync_kb_to_home.sh /path/to/proj_${PROJ}/home/kb
```

### Step E — 上线验收（必须）

#### E1 路由冒烟（约 4 题）

```bash
python3 scripts/gpos-manual-eval/route_smoke_271.py
# 需 export CLAW_ADMIN_TOKEN=...；可选 CLAW_ADMIN_MCP_URL / GPOS_MANUAL_KB
```

最低用例：

| 问法 | 期望 |
|------|------|
| How do I add a product in Back Office? | 手册 + **en** 链接 |
| 后台怎么连接厨房打印机？ | 手册 + **en** 链接 |
| เพิ่มสมาชิกในระบบหลังบ้านอย่างไร? | 手册 + **th** 链接 |
| Tell me a joke | self-introduction，无手册 URL |
| What were yesterday's sales? | 经营分析（THB 等），无手册 URL |

#### E2 全量 live（≥100，含多语种）

```bash
# 重建题集并跑 batch（默认 proj 271；上线生产前改脚本常量）
export CLAW_ADMIN_TOKEN=...
python3 scripts/gpos-manual-eval/run_live_core_271.py --min 100
```

门槛（与产品约定一致时可调整）：

| 指标 | 门槛 |
|------|------|
| 完成率 | 100% 跑完 |
| 通过率 | ≥ 90% |
| 语种链接正确 `url_lang_ok` | ≥ 95% |
| 错语种链接 `wrong_lang_url` | 尽量 0；预发已知残留见 LIVE_REPORT |
| 产品题误调 SQLBot | 0（抽检 transcript / 答复形态） |

产物：`eval/results.jsonl`、`summary.json`、`failures.md`、`LIVE_REPORT.md`。

#### E3 意图对照（小集）

见 [`docs/gpos-intent-routing-regress.md`](gpos-intent-routing-regress.md)。

---

## 4. 日常刷新（手册更新）

官网手册变更后：

```bash
# 1) 重爬
python3 scripts/gpos-manual-crawl/crawl_gpos_user_manual.py --lang all

# 2) 仅同步 KB（配置未变则不必 activate）
VER=$(ssh admin@192.168.9.250 \
  'readlink -f /data/claw-nas/pre-claw-01/proj_271/home/project_home_def')
rsync -az --delete --exclude eval/ --exclude README.md \
  knowledge/gpos-user-manual/ \
  "admin@192.168.9.250:${VER}/home/kb/"

# 3) 冒烟 + 可选全量
python3 scripts/gpos-manual-eval/route_smoke_271.py
```

**不要**假设 worker 启动会拉 Git。`git_sync` 若启用：仍须显式 `POST /v1/projects/{projId}/git/pull`，且 pull 目标目录须与当前物化布局一致（见 [`project-config-model.md`](project-config-model.md)）。

建议 cron：每日或官网发版后触发「爬取 → rsync → 冒烟」。

---

## 5. 生产上线（从预发 271 → 生产 27）

1. 预发 `LIVE_REPORT` / 回归清单全部达标。  
2. 在生产 Admin 对 **proj 27**（或灰度 id）执行 **Step C**（同构 API，`note` 标明生产发布人与原因）。  
3. **Step D** 同步 KB 到生产 NAS/workspace 的**当前** `project_home_def`。  
4. **Step E** 在生产用真实 `extraSession` 冒烟；全量可先灰度门店。  
5. BFF：确认 `projId` 指向已发布项目；协议字段不改。  
6. 观察：错语种链接率、手册题是否误进 SQLBot、分析题是否被手册截胡。

---

## 6. 回滚

### 6.1 配置回滚（推荐）

Admin / API：

```text
project_config_activate({ projId, contentRev: "<上一稳定版>" })
```

预发曾用稳定版示例：`2026-07-11_15-22-08`（回滚前确认该版**不含**错误技能覆盖）。

激活后：

- skills/CLAUDE/rules 回到旧版；
- **KB 不会自动带回**：若旧 version 目录无 kb，需再次 rsync，或接受无手册能力。

### 6.2 仅回滚 KB

对当前 `project_home_def` 重新 rsync 上一份已知良好的本地 `knowledge/gpos-user-manual/`（或其它备份目录）快照。

---

## 7. 故障排查

| 现象 | 可能原因 | 处理 |
|------|----------|------|
| 手册题答「查不到」/无链接 | activate 后未 rsync 到**新** version | 对当前 `project_home_def` 再 rsync |
| 泰文问给了 `/en/` 链接 | skill/CLAUDE 语种硬锁未生效或旧 version | 确认 active contentRev；检查 skill 文案；重建 worker 后再测 |
| 中/英问给了 `/th/` 链接 | 同上 | 同上；参考 LIVE_REPORT 失败样例 |
| 产品 how-to 变成经营报告 | 误进 SQLBot | 查 CLAUDE 三路意图；确认 `product-manual-qa` 已 activate |
| 诊断 skills 全没了 | skills draft 整表只写了 1～2 个 | 用备份整表恢复 → commit → activate |
| git/pull 无效果 | 未 enable / 未手动 pull / worker 未挂到更新路径 | 查 `gitSyncJson`；显式 pull；对 NAS version 核对 |
| solve 504 timeout | 模型/池繁忙 | 重试；避开高峰；适当增大 timeout |

查看当前生效版本：

```bash
# Admin MCP project_config_get 或：
curl -sS -H "Authorization: Bearer $TOKEN" \
  "$GATEWAY/v1/projects" | jq '.projects[] | select(.projId==271)'
ssh admin@192.168.9.250 \
  'readlink /data/claw-nas/pre-claw-01/proj_271/home/project_home_def'
```

---

## 8. 权限与安全

- Admin Token / PAT：仅运维持有，不入库明文（`gitPatId` 用全局 PAT）。  
- rsync / ssh：限定 `admin@` 到 NAS；勿对整个 `/data/claw-nas` 做无过滤 delete。  
- `eval/`、含 Token 的本地脚本：不要打进对外镜像；跑批脚本内 Token 上线前改为环境变量。

建议将 `route_smoke_271.py` / `run_live_core_271.py` 中的 Token 改为：

```bash
export CLAW_ADMIN_TOKEN=...
export CLAW_GATEWAY=http://192.168.9.252:18088
```

（脚本改造可后续 PR；当前预发验收曾使用内联 Token。）

---

## 9. 上线检查清单（打勾用）

- [ ] 双语爬取完成，`en_count` / `th_count` 正常，会员篇 en/th `source_url` 语种正确  
- [ ] fixtures / CLAUDE / Rules 已评审  
- [ ] Admin：**整表** skills + claude + rules → commit（有 note）→ activate  
- [ ] 记录 `savedContentRev` / `activeContentRev`  
- [ ] rsync KB → **当前** `project_home_def/home/kb`（exclude eval）  
- [ ] 抽检 NAS：`product-manual-qa` skill 存在；en/th 各一篇 `source_url`  
- [ ] 路由冒烟：手册 en / 手册 th / 闲聊 / 经营分析  
- [ ] （建议）全量 live ≥100，通过率与语种链接达标  
- [ ] 回滚 contentRev 已确认可用  
- [ ] 生产 BFF `projId` 已切（若需要）  

---

## 10. 禁止事项

1. 不要用旁路直接改 NAS 上 CLAUDE/skills 代替 Admin 发布（无版本记录）。  
2. 不要对 `project_skills_put_draft` 只提交新增 skill。  
3. 不要假设 worker 首次启动会 `git pull`。  
4. 不要把整本手册塞进 Skill 正文。  
5. 不要用大模型「翻译加工」后再当 KB 真源。  
6. 不要在 activate 之后忘记向**新 version** rsync KB。  

---

## 11. 预发已验证基线（参考）

截至 2026-07-13 预发 271：

- 生效配置曾至 `2026-07-13_06-48-55`  
- Live **105** 题（en/zh/th 各 35）：通过率约 **96.2%**；语种链接约 **97.1%**  
- 明细见本地 `knowledge/gpos-user-manual/eval/LIVE_REPORT.md`（不入库）或钉钉「Live 105 基线」文档

生产发布以**当次**验收为准，勿直接照搬历史通过率数字作为放行唯一依据。

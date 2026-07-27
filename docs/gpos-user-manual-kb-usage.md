# GPOS 产品手册知识库 · 完整使用说明

Author: kejiqing

本文说明手册 KB **从爬取到运行时检索**的整条用法：产物长什么样、怎么生成、怎么同步到 worker、助手怎么读、怎么验收。

**边界：** 业务 KB 正文**不进** claw-code。仓库只保留爬取 / 同步 / 评测脚本与 Admin fixtures；正文落本地 `knowledge/`（gitignore）或 NAS `home/kb`。

| 文档 | 用途 |
|------|------|
| **本文** | 日常使用与产物约定 |
| [`gpos-user-manual-kb-ops.md`](gpos-user-manual-kb-ops.md) | 预发/生产上线、回滚、故障排查（运维真源） |
| [`gpos-intent-routing-regress.md`](gpos-intent-routing-regress.md) | 三路意图回归清单 |
| [`scripts/gpos-manual-crawl/`](../scripts/gpos-manual-crawl/) | 爬取工具 |
| [`scripts/gpos-manual-eval/`](../scripts/gpos-manual-eval/) | 冒烟 / Live 评测 |

---

## 1. 能力在整包里的位置

```text
用户问题
  ├─ 闲聊 / 能力外     → Skill(self-introduction)     不查 KB
  ├─ GPOS 产品 how-to  → Skill(product-manual-qa)     查 home/kb，禁止 SQLBot
  └─ 经营问数 / 诊断   → 分析 skills + SQLBot         禁止用手册当数据答案
```

手册 KB 只服务中间一路：**产品操作 how-to**。

**语言路由（强制）**

| 用户输入 | KB 目录（worker） | 官方链接前缀 |
|----------|-------------------|--------------|
| 含泰文字符 | `/claw_ds/home/kb/th/` | `https://gpos.co.th/th/user-manual/` |
| 其他（中 / 英 / …） | `/claw_ds/home/kb/en/` | `https://gpos.co.th/en/user-manual/` |

- 正文：**只爬取官网原文**，禁止用大模型翻译/改写后再当 KB。
- 答复：摘 3–8 条步骤 + 该页 `source_url`（语种必须与路由一致）。

---

## 2. 端到端流程（一张图）

```text
官网 gpos.co.th/{en,th}/user-manual
        │
        ▼  crawl_gpos_user_manual.py
knowledge/gpos-user-manual/     ← 本地缓存（gitignore）
  index.md / manifest.json
  en/**/*.md
  th/**/*.md
        │
        ▼  rsync / sync_kb_to_home.sh
NAS 当前 project_home_def/home/kb/   （或本地 proj_N/home/kb）
        │
        ▼  worker 挂载
/claw_ds/home/kb/{en,th}/
        │
        ▼  Skill product-manual-qa
grep_search / read_file → 摘步骤 + source_url
```

要点：

1. **爬取**产出本地缓存。  
2. **同步**到当前生效的 `project_home_def`（activate 会换 version，必须对**当前**路径 rsync）。  
3. **配置**（CLAUDE / skills）走 Admin draft → commit → activate，与 KB 文件路径分离。  
4. **评测**读同一套 md 或打网关 live。

---

## 3. 产物目录与文件格式

### 3.1 目录树

默认根目录：`knowledge/gpos-user-manual/`（可用环境变量 `GPOS_MANUAL_KB` 覆盖）。

```text
knowledge/gpos-user-manual/
├── index.md                 # 根索引（双语入口说明）
├── manifest.json            # 总清单：en_count / th_count / page_count
├── en/
│   ├── index.md             # 英文分类索引
│   ├── manifest.json        # 英文页清单
│   ├── getting-started.md
│   └── membership/
│       └── add-member-back-office.md
├── th/
│   ├── index.md
│   ├── manifest.json
│   └── …（与 en 对称的相对路径）
└── eval/                    # 评测产物（勿 rsync 到运行时 kb）
    ├── results.jsonl
    ├── summary.json
    └── LIVE_REPORT.md
```

同步到运行时时 **exclude** `eval/`、`README.md`。

### 3.2 单页 Markdown（核心处理结果）

每篇手册页由爬虫 `render_md()` 写出：YAML frontmatter + 原文步骤块。**无 LLM 改写**。

```markdown
---
title: Add Member (Back Office)
source_url: https://gpos.co.th/en/user-manual/membership/add-member-back-office
lang: en
category: Membership
category_slug: membership
keywords: [member, add, back office, membership]
crawled_at: 2026-07-13T06:48:55Z
---

# Add Member (Back Office)

**Official docs:** https://gpos.co.th/en/user-manual/membership/add-member-back-office

## Steps / Content

1. Go to Back Office → Membership.
2. Click Add Member.
3. Fill in required fields and save.

## Keywords

member, add, back office, membership

<!-- Author: kejiqing; lang=en; crawled_at=2026-07-13T06:48:55Z -->
```

字段约定：

| 字段 | 含义 |
|------|------|
| `title` | 页面标题 |
| `source_url` | 官网原文 URL（答复必须带此链接，且语种与 `lang` 一致） |
| `lang` | `en` 或 `th` |
| `category` / `category_slug` | 手册大类 |
| `keywords` | 从标题/正文抽的检索辅助词 |
| `crawled_at` | UTC 爬取时间 |
| `## Steps / Content` | 可抽取正文；抽不到则提示打开官网 |

泰文页结构相同，例如：

- `lang: th`
- `source_url: https://gpos.co.th/th/user-manual/membership/add-member-back-office`
- 相对路径仍为 `th/membership/add-member-back-office.md`

### 3.3 根 `manifest.json` 示例

```json
{
  "crawled_at": "2026-07-13T06:48:55Z",
  "languages": ["en", "th"],
  "page_count": 240,
  "en_count": 120,
  "th_count": 120
}
```

语种目录下另有 `en/manifest.json` / `th/manifest.json`，含逐页 `title`、`source_url`、`path`、`body_len` 等。

### 3.4 根 `index.md` 作用

Worker 可先读 `/claw_ds/home/kb/index.md` 确认双语入口，再进入 `en/` 或 `th/` 检索。

---

## 4. 爬取（生成本地 KB）

### 4.1 命令

```bash
cd /path/to/claw-code

# 双语全量（推荐）
python3 scripts/gpos-manual-crawl/crawl_gpos_user_manual.py --lang all --delay 0.2

# 只爬一种语言 / 限页（调试）
python3 scripts/gpos-manual-crawl/crawl_gpos_user_manual.py --lang en --max-pages 5
```

| 参数 / 环境变量 | 说明 |
|-----------------|------|
| `--lang` | `en` / `th` / `all`（默认 `all`） |
| `--out` | 输出根；默认 `knowledge/gpos-user-manual` |
| `--delay` | 请求间隔秒（默认 `0.2`） |
| `--max-pages` | 每语种最大页数；调试用 |
| `GPOS_MANUAL_KB` | 覆盖默认输出根（与 `--out` 同类） |

### 4.2 抽检

```bash
python3 - <<'PY'
import json
from pathlib import Path
root = Path("knowledge/gpos-user-manual")
m = json.loads((root / "manifest.json").read_text(encoding="utf-8"))
print(m)
assert m.get("en_count", 0) > 100 and m.get("th_count", 0) > 100
assert (root / "en/membership/add-member-back-office.md").exists()
assert (root / "th/membership/add-member-back-office.md").exists()
print("kb ok")
PY
```

抽检 `source_url` 语种：

```bash
grep -m1 source_url knowledge/gpos-user-manual/en/membership/add-member-back-office.md
grep -m1 source_url knowledge/gpos-user-manual/th/membership/add-member-back-office.md
```

期望分别含 `/en/user-manual/` 与 `/th/user-manual/`。

---

## 5. 同步到运行时 `home/kb`

### 5.1 本地 / 开发机

```bash
scripts/gpos-manual-crawl/sync_kb_to_home.sh /path/to/proj_N/home/kb
```

等价于 `rsync -a --delete`，并排除 `eval/`、`README.md`、`.git/`。源目录默认 `knowledge/gpos-user-manual`，可用 `GPOS_MANUAL_KB` 覆盖。

### 5.2 预发 NAS（activate 之后）

每次 Admin **activate** 会切到新的 `contentRev` 目录。KB 必须 rsync 到**当前** `project_home_def`，否则 worker 读到空 KB。

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
```

Worker 侧逻辑路径：

| 逻辑路径 | 含义 |
|----------|------|
| `/claw_ds/home/kb/index.md` | 根索引 |
| `/claw_ds/home/kb/en/` | 英/中等问题检索根 |
| `/claw_ds/home/kb/th/` | 泰文问题检索根 |

### 5.3 日常刷新（官网手册更新）

```bash
python3 scripts/gpos-manual-crawl/crawl_gpos_user_manual.py --lang all
# 再对当前 project_home_def 执行上一节 rsync
# 配置未改则不必重新 activate
```

---

## 6. 配置侧（Skill / CLAUDE）怎么用 KB

KB 文件本身不进 Admin；Admin 发布的是路由与 skill 文案。

关键 skill fixture：[`scripts/fixtures/skills/product-manual-qa.SKILL.md`](../scripts/fixtures/skills/product-manual-qa.SKILL.md)

检索协议（摘要）：

1. 按语种选定 `KB_LANG_ROOT`（`.../kb/th` 或 `.../kb/en`）。  
2. 可选 `read_file` → `$KB_LANG_ROOT/index.md`。  
3. `grep_search(path=$KB_LANG_ROOT, pattern=<关键词>)`。  
4. `read_file` 命中文章；使用 frontmatter 的 `source_url`。  
5. 0 命中：`glob_search` 后再读；仍无则回对应语种手册首页链接。

对用户输出结构：一句结论 → 3–8 条步骤 → **必须**带正确语种的 `source_url`。禁止暴露 `/claw_ds` 等内部路径。

发布顺序（细节与整表覆盖陷阱见运维手册）：

1. `project_config_get` 拉齐现网  
2. **整表**合并后 `project_skills_put_draft` / claude / rules  
3. `project_config_commit_draft` → `savedContentRev`  
4. `project_config_activate` → 再 **rsync KB** 到新 version

---

## 7. 评测与验收

环境变量：

| 变量 | 含义 |
|------|------|
| `GPOS_MANUAL_KB` | 本地 KB 根 |
| `GPOS_MANUAL_EVAL_OUT` | 跑批产物（默认 `$GPOS_MANUAL_KB/eval`） |
| `CLAW_ADMIN_TOKEN` | Live / 冒烟必填 |
| `CLAW_ADMIN_MCP_URL` | Admin MCP（默认预发） |

```bash
export CLAW_ADMIN_TOKEN=...

# 路由冒烟（手册 en / 手册 th / 闲聊 / 经营）
python3 scripts/gpos-manual-eval/route_smoke_271.py

# 全量 live（建议 ≥100）
python3 scripts/gpos-manual-eval/run_live_core_271.py --min 100
```

Live 产物在 `eval/`：`results.jsonl`、`summary.json`、`failures.md`、`LIVE_REPORT.md`。

建议门槛（可与产品约定调整）：完成率 100%；通过率 ≥ 90%；语种链接正确率 ≥ 95%；产品题误调 SQLBot = 0。

---

## 8. 禁止事项（使用侧）

1. 不要把业务手册正文 commit 进 claw-code。  
2. 不要用大模型「翻译加工」后再当 KB 真源。  
3. 不要把整本手册塞进 Skill 正文。  
4. 不要在 activate 后忘记向**新** `project_home_def` rsync KB。  
5. 不要对 `project_skills_put_draft` 只提交新增 skill（整表覆盖会丢掉其它 skills）。  
6. 不要假设 worker 启动会自动 `git pull`；KB 以显式 rsync / 约定拉取为准。

---

## 9. 最小上手清单

- [ ] `crawl_gpos_user_manual.py --lang all` 成功，`manifest.json` 双语计数正常  
- [ ] 打开任意 `en/`、`th/` 下 md，确认 frontmatter + `Steps / Content`  
- [ ] Admin 已发布 `product-manual-qa`（整表 merge）并 activate  
- [ ] rsync 到当前 `project_home_def/home/kb`（exclude `eval/`）  
- [ ] 冒烟：英/中手册题给 en 链接；泰文题给 th 链接；闲聊与问数不走手册  

上线、回滚与故障表见 [`gpos-user-manual-kb-ops.md`](gpos-user-manual-kb-ops.md)。

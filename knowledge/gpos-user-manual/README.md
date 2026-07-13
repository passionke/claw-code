# GPOS User Manual KB

Author: kejiqing

Static product how-to corpus for QueryX **product-manual-qa**.

## Layout

```text
knowledge/gpos-user-manual/
├── index.md                 # bilingual entry
├── en/                      # English crawl → gpos.co.th/en/user-manual/...
│   ├── index.md
│   ├── manifest.json
│   └── <category>/<slug>.md
├── th/                      # Thai crawl → gpos.co.th/th/user-manual/...
│   ├── index.md
│   ├── manifest.json
│   └── <category>/<slug>.md
└── eval/
```

Runtime: `/claw_ds/home/kb/{en,th}/`

## Language routing

| User input | KB | Official URL |
|------------|-----|--------------|
| Thai | `th/` | `https://gpos.co.th/th/user-manual/...` |
| Other | `en/` | `https://gpos.co.th/en/user-manual/...` |

Raw crawl only — no LLM rewrite of manual pages.

## Refresh / Go-live

```bash
python3 scripts/gpos-manual-crawl/crawl_gpos_user_manual.py --lang all
```

**上线运维（发布、NAS rsync、验收、回滚）：**  
[`docs/gpos-user-manual-kb-ops.md`](../docs/gpos-user-manual-kb-ops.md)

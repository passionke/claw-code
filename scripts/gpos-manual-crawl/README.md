# gpos-manual-crawl

Crawl GPOS EN + TH user manuals into **local** `knowledge/gpos-user-manual/{en,th}/`.

该目录在 `.gitignore` 中：**业务 KB 不进 claw-code**，只作爬取缓存 / rsync 源，运行时落 NAS `home/kb`。

```bash
python3 scripts/gpos-manual-crawl/crawl_gpos_user_manual.py --lang all
```

可选：`GPOS_MANUAL_KB` 覆盖默认输出根目录。

**上线运维：** [`docs/gpos-user-manual-kb-ops.md`](../../docs/gpos-user-manual-kb-ops.md)  
**评测：** [`scripts/gpos-manual-eval/`](../gpos-manual-eval/)

Author: kejiqing

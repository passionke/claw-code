#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""Materialize Mind FAQ folder exports into home/kb/en/internal/mind/faq/. Author: kejiqing"""

from __future__ import annotations

import json
import re
import sys
from datetime import datetime, timezone
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
MANIFEST = Path(__file__).with_name("mind_faq_manifest.json")
EXPORT_DIR = ROOT / "knowledge/gpos-user-manual/.mind-export/faq"
KB_ROOT = Path(
    __import__("os").environ.get("GPOS_MANUAL_KB", str(ROOT / "knowledge/gpos-user-manual"))
)
OUT_ROOT = KB_ROOT / "en" / "internal" / "mind" / "faq"


def slug(text: str) -> str:
    s = re.sub(r"[^\w\u0E00-\u0E7F\u4e00-\u9fff-]+", "-", text.strip().lower())
    return re.sub(r"-+", "-", s).strip("-") or "doc"


def main() -> int:
    if not MANIFEST.exists():
        print(f"missing {MANIFEST}", file=sys.stderr)
        return 2
    spec = json.loads(MANIFEST.read_text(encoding="utf-8"))
    folder_url = spec.get("folder_url", "")
    now = datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
    OUT_ROOT.mkdir(parents=True, exist_ok=True)
    index_lines = [
        "# GPOS FAQ（内部补充，不对商家暴露 Mind 链接）",
        "",
        "Author: kejiqing",
        "",
        "中文 FAQ，与官网 en/th 手册并列检索。`source_url` 为内部标识，**禁止**写入用户可见答复。",
        "",
    ]
    ok = 0
    missing = []
    for doc in spec.get("docs", []):
        rid = doc["id"]
        title = doc.get("title") or rid
        category = doc.get("category") or "其他"
        raw_path = EXPORT_DIR / f"{rid}.md"
        if not raw_path.exists():
            missing.append(rid)
            continue
        body = raw_path.read_text(encoding="utf-8").strip()
        cat_slug = slug(category)
        name = slug(title) + ".md"
        out = OUT_ROOT / cat_slug / name
        out.parent.mkdir(parents=True, exist_ok=True)
        internal_ref = f"internal:faq/{cat_slug}/{name}"
        wrapped = f"""---
title: {title}
source_url: {internal_ref}
lang: zh
category: {category}
category_slug: {cat_slug}
keywords: [gpos, faq, {category}, internal]
crawled_at: {now}
origin: mind
public_citation: false
---

# {title}

## Steps / Content

{body}
"""
        out.write_text(wrapped, encoding="utf-8")
        index_lines.append(f"- [{category}] {title} → `{cat_slug}/{name}`")
        ok += 1
    (OUT_ROOT / "index.md").write_text("\n".join(index_lines) + "\n", encoding="utf-8")
    print(f"faq exported: {ok}, missing raw: {len(missing)}")
    if missing:
        print("missing ids:", ", ".join(missing[:5]), "..." if len(missing) > 5 else "", file=sys.stderr)
        return 1 if ok == 0 else 0
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

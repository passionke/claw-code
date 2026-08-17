#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""Copy Git + Mind internal supplement into knowledge/gpos-user-manual/en/internal/. Author: kejiqing"""

from __future__ import annotations

import json
import re
import sys
from datetime import datetime, timezone
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
KB_ROOT = Path(
    __import__("os").environ.get("GPOS_MANUAL_KB", str(ROOT / "knowledge/gpos-user-manual"))
)
INTERNAL = KB_ROOT / "en" / "internal"
MANIFEST = Path(__file__).with_name("internal_kb_manifest.json")


def slug(title: str) -> str:
    s = re.sub(r"[^\w\u0E00-\u0E7F\u4e00-\u9fff-]+", "-", title.strip().lower())
    return re.sub(r"-+", "-", s).strip("-") or "doc"


def wrap_git_doc(src: Path, title: str, source_url: str) -> str:
    body = src.read_text(encoding="utf-8")
    now = datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
    return f"""---
title: {title}
source_url: {source_url}
lang: en
category: Internal Supplement
category_slug: internal
keywords: [gpos, internal, supplement]
crawled_at: {now}
origin: git
---

# {title}

{body}
"""


def main() -> int:
    if not MANIFEST.exists():
        print(f"missing manifest: {MANIFEST}", file=sys.stderr)
        return 2
    spec = json.loads(MANIFEST.read_text(encoding="utf-8"))
    INTERNAL.mkdir(parents=True, exist_ok=True)
    (INTERNAL / "index.md").write_text(
        """# Internal supplement (Git + Mind)

Author: kejiqing

内部补充文档，与 `en/` 官方手册并列检索。优先官方 `source_url`；internal 仅作补充。
""",
        encoding="utf-8",
    )
    count = 0
    for item in spec.get("git_docs", []):
        rel = item["path"]
        src = ROOT / rel
        if not src.exists():
            print(f"skip missing git doc: {rel}", file=sys.stderr)
            continue
        name = slug(item.get("title") or src.stem) + ".md"
        out = INTERNAL / "claw" / name
        out.parent.mkdir(parents=True, exist_ok=True)
        out.write_text(
            wrap_git_doc(src, item.get("title") or src.stem, item.get("source_url", f"git:{rel}")),
            encoding="utf-8",
        )
        count += 1
        print(f"wrote {out}")
    for item in spec.get("mind_docs", []):
        # Pre-exported markdown files (run export_mind_kb_supplement.sh to refresh from MCP).
        rel = item.get("local_markdown")
        if not rel:
            continue
        src = ROOT / rel
        if not src.exists():
            print(f"skip missing mind export: {rel}", file=sys.stderr)
            continue
        name = slug(item.get("title") or Path(rel).stem) + ".md"
        out = INTERNAL / "mind" / name
        out.parent.mkdir(parents=True, exist_ok=True)
        body = src.read_text(encoding="utf-8")
        if not body.lstrip().startswith("---"):
            now = datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
            url = item.get("mind_url", f"mind:{item.get('resource_id', '')}")
            body = f"""---
title: {item.get('title', name)}
source_url: {url}
lang: en
category: Internal Supplement
category_slug: internal
keywords: [gpos, mind, supplement]
crawled_at: {now}
origin: mind
---

{body}
"""
        out.write_text(body, encoding="utf-8")
        count += 1
        print(f"wrote {out}")
    print(f"internal supplement files: {count}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

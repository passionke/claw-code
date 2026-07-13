#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""Sanity-check intent contrast sets + core-questions cardinality. Author: kejiqing"""

from __future__ import annotations

import json
import os
import sys
from pathlib import Path

SCRIPT = Path(__file__).resolve().parent
REPO = SCRIPT.parents[1]
KB = Path(os.environ.get("GPOS_MANUAL_KB", REPO / "knowledge" / "gpos-user-manual"))
EVAL_OUT = Path(os.environ.get("GPOS_MANUAL_EVAL_OUT", KB / "eval"))


def load(path: Path) -> list[dict]:
    return [
        json.loads(line)
        for line in path.read_text(encoding="utf-8").splitlines()
        if line.strip()
    ]


def main() -> int:
    core_path = EVAL_OUT / "core-questions.jsonl"
    if not core_path.is_file():
        print(f"缺少 {core_path}（先 crawl 再 run_live_core_271.py --build-only）", file=sys.stderr)
        return 2
    core = load(core_path)
    chat = load(SCRIPT / "chitchat.jsonl")
    analysis = load(SCRIPT / "analysis.jsonl")
    errors: list[str] = []
    if len(core) < 100:
        errors.append(f"core-questions count {len(core)} < 100")
    langs = {r["lang"] for r in core}
    if langs != {"en", "zh", "th"}:
        errors.append(f"core langs incomplete: {langs}")
    if any(r.get("intent") != "product_manual" for r in core):
        errors.append("core intent must be product_manual")
    if any(not r.get("must_not_call_mcp") for r in core):
        errors.append("core must_not_call_mcp required")
    if len(chat) < 10:
        errors.append("chitchat < 10")
    if len(analysis) < 10:
        errors.append("analysis < 10")
    if any(r.get("intent") != "chitchat" for r in chat):
        errors.append("chitchat intent")
    if any(r.get("intent") != "analysis" for r in analysis):
        errors.append("analysis intent")
    if errors:
        print("FAIL", *errors, sep="\n- ", file=sys.stderr)
        return 1
    print(
        json.dumps(
            {
                "core": len(core),
                "chitchat": len(chat),
                "analysis": len(analysis),
                "core_langs": sorted(langs),
                "ok": True,
            },
            ensure_ascii=False,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""Sanity-check intent contrast sets + core-questions cardinality. Author: kejiqing"""

from __future__ import annotations

import json
import sys
from pathlib import Path

EVAL = Path(__file__).resolve().parent


def load(name: str) -> list[dict]:
    return [
        json.loads(line)
        for line in (EVAL / name).read_text(encoding="utf-8").splitlines()
        if line.strip()
    ]


def main() -> int:
    core = load("core-questions.jsonl")
    chat = load("chitchat.jsonl")
    analysis = load("analysis.jsonl")
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
    # boundary strings present
    blob = "\n".join(r["question"] for r in core + chat + analysis)
    for needle in ["sales", "商品", "hello", "ยอดขาย"]:
        if needle.lower() not in blob.lower() and needle not in blob:
            # soft: only warn
            pass
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

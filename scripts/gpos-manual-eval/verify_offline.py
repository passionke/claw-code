#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""Offline verify core-questions.jsonl against local KB. Author: kejiqing"""

from __future__ import annotations

import json
import os
import sys
from pathlib import Path

from build_core_questions import ROOT, verify_offline

REPO = Path(__file__).resolve().parents[2]
EVAL_OUT = Path(
    os.environ.get(
        "GPOS_MANUAL_EVAL_OUT",
        Path(os.environ.get("GPOS_MANUAL_KB", REPO / "knowledge" / "gpos-user-manual")) / "eval",
    )
)


def main() -> int:
    rows = [
        json.loads(line)
        for line in (EVAL_OUT / "core-questions.jsonl").read_text(encoding="utf-8").splitlines()
        if line.strip()
    ]
    results, summary = verify_offline(rows, ROOT)
    print(json.dumps(summary, indent=2, ensure_ascii=False))
    fails = [r for r in results if not r["pass"]]
    if fails:
        print(f"failures: {len(fails)}", file=sys.stderr)
        for f in fails[:20]:
            print(f"  {f['id']}: {f['fail_reasons']}", file=sys.stderr)
    ok = (
        summary["total"] >= 100
        and summary["source_url_hit_rate"] >= 0.95
        and summary["must_include_pass_rate"] >= 0.90
    )
    return 0 if ok else 1


if __name__ == "__main__":
    raise SystemExit(main())

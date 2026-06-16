#!/usr/bin/env python3
"""Push GPOS language config from pre-prod proj27 to production proj10.

Syncs:
  - languagePipelineJson (GPOS user-message language rules)
  - claudeMd (Thai anti-CJK patches v1–v3, idempotent)

Author: kejiqing
"""

from __future__ import annotations

import argparse
import json
import sys
import urllib.error
import urllib.request

from patch_proj27_claude_thai import patch_claude_md

PRE = "http://192.168.9.252:18088"
PRE_PROJ = 27
PROD = "http://10.200.2.171:18088"
PROD_PROJ = 10


def req(method: str, base: str, path: str, body: dict | None = None) -> dict:
    data = json.dumps(body, ensure_ascii=False).encode() if body is not None else None
    r = urllib.request.Request(
        f"{base}{path}",
        data=data,
        method=method,
        headers={"Content-Type": "application/json"},
    )
    try:
        with urllib.request.urlopen(r, timeout=60) as resp:
            return json.loads(resp.read().decode())
    except urllib.error.HTTPError as e:
        raise SystemExit(f"{method} {base}{path} -> HTTP {e.code}: {e.read().decode()[:600]}")


def build_payload(cur: dict, *, claude_md: str, language_pipeline: dict) -> dict:
    payload = {
        "rulesJson": cur["rulesJson"],
        "mcpServersJson": cur["mcpServersJson"],
        "skillsSourcesJson": cur.get("skillsSourcesJson") or [],
        "skillsJson": cur["skillsJson"],
        "allowedToolsJson": cur["allowedToolsJson"],
        "claudeMd": claude_md,
        "languagePipelineJson": language_pipeline,
    }
    for k in (
        "gitSyncJson",
        "solvePreflightJson",
        "solveOrchestrationJson",
        "extraSessionFieldsJson",
        "promptLimitsJson",
        "workerIsolationJson",
    ):
        if k in cur and cur[k] is not None:
            payload[k] = cur[k]
    return payload


def verify_markers(md: str, lp: dict) -> dict[str, bool]:
    return {
        "gpos_language_rules": "store_name" in (lp.get("languageInferencePrompt") or ""),
        "language_lock": "#### Language lock" in md,
        "traffic_footnote_th": "ข้อมูลปริมาณลูกค้าไม่พร้อมใช้งาน" in md,
        "traffic_glossary": "泰文禁止借词 `客流`" in md,
        "borrow_scan": "**借词扫描（Thai）**" in md,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description="Push GPOS lang config pre27 -> prod10")
    parser.add_argument("--dry-run", action="store_true", help="preview only, no PUT")
    args = parser.parse_args()

    pre = req("GET", PRE, f"/v1/project/config/{PRE_PROJ}")
    prod = req("GET", PROD, f"/v1/project/config/{PROD_PROJ}")

    lp = pre.get("languagePipelineJson")
    if not lp or "store_name" not in (lp.get("languageInferencePrompt") or ""):
        raise SystemExit("pre-prod languagePipelineJson missing GPOS rules")

    md = prod.get("claudeMd") or ""
    patched, applied = patch_claude_md(md)
    if not applied:
        print("claudeMd: already patched (no v1–v3 deltas)")
    else:
        print(f"claudeMd chars: {len(md)} -> {len(patched)}")
        print("patches applied:", ", ".join(applied))

    before = verify_markers(md, prod.get("languagePipelineJson") or {})
    after = verify_markers(patched, lp)
    print("before prod:", before)
    print("after push:", after)

    if args.dry_run:
        print("dry-run: skip PUT/commit/activate")
        return 0

    payload = build_payload(prod, claude_md=patched, language_pipeline=lp)
    req("PUT", PROD, f"/v1/project/config/{PROD_PROJ}", payload)
    commit = req(
        "POST",
        PROD,
        f"/v1/project/config/{PROD_PROJ}/versions/commit",
        {"note": "GPOS language pipeline + Thai anti-CJK CLAUDE.md (from pre27, kejiqing)"},
    )
    rev = commit["savedContentRev"]
    act = req("POST", PROD, f"/v1/project/config/{PROD_PROJ}/versions/{rev}/activate")
    print("activated", act.get("activated"), "rev", rev)

    verify = req("GET", PROD, f"/v1/project/config/{PROD_PROJ}")
    print("verify:", verify_markers(verify.get("claudeMd") or "", verify.get("languagePipelineJson") or {}))
    print("stableContentRev", verify.get("stableContentRev"))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""Route smoke tests against pre proj 271 via Admin MCP HTTP. Author: kejiqing"""

from __future__ import annotations

import json
import os
import re
import sys
import time
import urllib.request
from pathlib import Path

URL = os.environ.get("CLAW_ADMIN_MCP_URL", "http://192.168.9.252:18088/v1/admin/mcp")
TOKEN = os.environ.get("CLAW_ADMIN_TOKEN", "").strip()
if not TOKEN:
    print("缺少环境变量 CLAW_ADMIN_TOKEN", file=sys.stderr)
    sys.exit(2)
EXTRA = {
    "store_id": "S002501221841976200006188",
    "store_name": "G&G Ratchaburi",
    "org_id": "",
}
OUT = Path(__file__).resolve().parent / "route-smoke-results.jsonl"


def mcp_call(name: str, arguments: dict, timeout: int = 300) -> dict:
    payload = {
        "jsonrpc": "2.0",
        "id": int(time.time() * 1000) % 1_000_000_000,
        "method": "tools/call",
        "params": {"name": name, "arguments": arguments},
    }
    req = urllib.request.Request(
        URL,
        data=json.dumps(payload).encode("utf-8"),
        headers={
            "Authorization": f"Bearer {TOKEN}",
            "Content-Type": "application/json",
            "Accept": "application/json, text/event-stream",
        },
        method="POST",
    )
    with urllib.request.urlopen(req, timeout=timeout) as resp:
        body = json.loads(resp.read().decode("utf-8"))
    if body.get("error"):
        raise RuntimeError(body["error"])
    content = body.get("result", {}).get("content") or []
    text = ""
    for c in content:
        if isinstance(c, dict) and c.get("type") == "text":
            text += c.get("text") or ""
    if not text:
        return body.get("result") or body
    try:
        return json.loads(text)
    except json.JSONDecodeError:
        return {"raw": text, "result": body.get("result")}


def http_get(path: str) -> dict:
    req = urllib.request.Request(
        f"http://192.168.9.252:18088{path}",
        headers={"Authorization": f"Bearer {TOKEN}"},
    )
    with urllib.request.urlopen(req, timeout=60) as resp:
        return json.loads(resp.read().decode("utf-8"))


def extract_message(solve: dict) -> str:
    oj = solve.get("outputJson") or {}
    if isinstance(oj, str):
        try:
            oj = json.loads(oj)
        except json.JSONDecodeError:
            oj = {}
    msg = oj.get("message") if isinstance(oj, dict) else None
    if msg:
        return str(msg)
    ot = solve.get("outputText") or ""
    try:
        return str(json.loads(ot).get("message") or ot)
    except Exception:
        return ot


CASES = [
    {
        "id": "route-product-add-item",
        "expect": "product_manual",
        "userPrompt": "How do I add a product in Back Office?",
        "timeoutSeconds": 180,
    },
    {
        "id": "route-product-printer-zh",
        "expect": "product_manual",
        "userPrompt": "后台怎么连接厨房打印机？",
        "timeoutSeconds": 180,
    },
    {
        "id": "route-chitchat-joke",
        "expect": "chitchat",
        "userPrompt": "Tell me a joke",
        "timeoutSeconds": 120,
    },
    {
        "id": "route-analysis-sales",
        "expect": "analysis",
        "userPrompt": "What were yesterday's sales?",
        "timeoutSeconds": 300,
    },
]


def classify_result(expect: str, message: str, solve: dict) -> dict:
    low = message.lower()
    url_hit = bool(
        re.search(r"https?://gpos\.co\.th/en/user-manual[^\s)\]\"']*", message)
    )
    # self-intro cues
    intro_cues = [
        "restaurant operations assistant",
        "经营助手",
        "ผู้ช่วย",
        "what were yesterday",
        "昨天的销售",
        "ยอดขายเมื่อวาน",
    ]
    looks_intro = any(c.lower() in low for c in intro_cues) and not url_hit
    # analysis cues: numbers / THB / table-ish / sales total
    analysis_cues = [
        "thb",
        "฿",
        "sales",
        "revenue",
        "ยอดขาย",
        "销售额",
        "|",
    ]
    looks_analysis = sum(1 for c in analysis_cues if c.lower() in low) >= 2

    verdict = {
        "expect": expect,
        "url_hit": url_hit,
        "looks_intro": looks_intro,
        "looks_analysis": looks_analysis,
        "message_preview": message[:400],
        "sessionId": solve.get("sessionId"),
        "durationMs": solve.get("durationMs"),
        "clawExitCode": solve.get("clawExitCode"),
    }
    if expect == "product_manual":
        verdict["pass"] = url_hit and not looks_intro
        verdict["reason"] = "need official manual URL in answer"
    elif expect == "chitchat":
        verdict["pass"] = looks_intro and not url_hit
        verdict["reason"] = "need self-introduction style, no manual URL required"
    else:
        verdict["pass"] = looks_analysis and not url_hit
        verdict["reason"] = "need analytics answer without manual URL"
    return verdict


def main() -> int:
    rows = []
    for case in CASES:
        print(f"RUN {case['id']} ...", flush=True)
        t0 = time.time()
        try:
            solve = mcp_call(
                "gateway_solve",
                {
                    "projId": 271,
                    "userPrompt": case["userPrompt"],
                    "extraSession": EXTRA,
                    "timeoutSeconds": case["timeoutSeconds"],
                },
                timeout=case["timeoutSeconds"] + 60,
            )
            msg = extract_message(solve)
            verdict = classify_result(case["expect"], msg, solve)
            verdict["id"] = case["id"]
            verdict["ok"] = True
            verdict["elapsedSec"] = round(time.time() - t0, 1)
        except Exception as e:
            verdict = {
                "id": case["id"],
                "expect": case["expect"],
                "pass": False,
                "ok": False,
                "error": str(e),
                "elapsedSec": round(time.time() - t0, 1),
            }
        print(json.dumps({k: verdict.get(k) for k in ("id", "pass", "url_hit", "looks_intro", "looks_analysis", "error", "sessionId", "elapsedSec")}, ensure_ascii=False))
        rows.append(verdict)
        time.sleep(1)
    OUT.write_text(
        "".join(json.dumps(r, ensure_ascii=False) + "\n" for r in rows),
        encoding="utf-8",
    )
    summary = {
        "total": len(rows),
        "passed": sum(1 for r in rows if r.get("pass")),
        "failed": [r["id"] for r in rows if not r.get("pass")],
    }
    (OUT.parent / "route-smoke-summary.json").write_text(
        json.dumps(summary, ensure_ascii=False, indent=2) + "\n", encoding="utf-8"
    )
    print("SUMMARY", json.dumps(summary, ensure_ascii=False))
    return 0 if summary["passed"] == summary["total"] else 1


if __name__ == "__main__":
    raise SystemExit(main())

#!/usr/bin/env python3
"""Acceptance: BOSS sales / PR SQL routing across languages. Author: kejiqing"""

from __future__ import annotations

import json
import os
import re
import sys
import time
import urllib.error
import urllib.request
from dataclasses import dataclass
from typing import Any

GATEWAY = os.environ.get("GATEWAY", "http://192.168.9.252:18088").rstrip("/")
PROJ_ID = int(os.environ.get("PROJ_ID", "27"))
STORE_ID = os.environ.get("STORE_ID", "S20241026154900007208")
POLL_SEC = float(os.environ.get("POLL_SEC", "5"))
TIMEOUT_SEC = int(os.environ.get("TIMEOUT_SEC", "180"))

EXTRA = {
    "tenant_code": "GPOS",
    "solution_code": "restaurant",
    "biz_type": "BOSS_REPORT",
    "store_id": STORE_ID,
    "store_name": " TAPAS Cafe & Restaurant Phuket",
    "org_id": " ",
    "_claw_client_origin": "metric-routing-accept",
}

FORBIDDEN = re.compile(
    r"\bSUM\s*\(\s*(?:t\d+\.)?sales_amount\s*\)|"
    r"\bSUM\s*\(\s*(?:t\d+\.)?gross_revenue\s*\)|"
    r"\bSUM\s*\(\s*CAST\s*\(\s*pay_amount\b",
    re.I,
)


@dataclass(frozen=True)
class Case:
    lang: str
    metric: str
    prompt: str


CASES = [
    Case("th", "sales", "เดือนนี้ยอดขายเท่าไหร่"),
    Case("zh", "sales", "这个月销售额是多少"),
    Case("en", "sales", "What is total sales this month?"),
    Case("my", "sales", "ဒီလရောင်းအားဘယ်လောက်ရှိပါသလဲ"),
    Case("th", "pr", "เดือนนี้ Payments Received เท่าไหร่"),
    Case("zh", "pr", "这个月 Payments Received 是多少"),
    Case("en", "pr", "What is Payments Received this month?"),
    Case("my", "pr", "ဒီလ Payments Received ဘယ်လောက်ရှိပါသလဲ"),
]


def http_json(method: str, path: str, body: dict | None = None, timeout: int = 60) -> Any:
    data = json.dumps(body, ensure_ascii=False).encode() if body is not None else None
    req = urllib.request.Request(
        f"{GATEWAY}{path}",
        data=data,
        headers={"Content-Type": "application/json"},
        method=method,
    )
    with urllib.request.urlopen(req, timeout=timeout) as resp:
        return json.loads(resp.read().decode())


def extract_sql(tools_payload: dict) -> str:
    for t in tools_payload.get("tools") or []:
        name = (t.get("toolName") or "").lower()
        if "question" not in name:
            continue
        raw = t.get("output") or ""
        if not isinstance(raw, str):
            continue
        try:
            outer = json.loads(raw)
            text = outer["content"][0]["text"]
            inner = json.loads(text)
            sql = inner.get("query", {}).get("sql")
            if isinstance(sql, str) and sql.strip():
                return sql
        except (KeyError, json.JSONDecodeError, TypeError, IndexError):
            continue
    return ""


def judge(metric: str, sql: str) -> tuple[bool, str]:
    if not sql:
        return False, "no SQL from sqlbot tool"
    s = sql.lower()
    if FORBIDDEN.search(sql):
        return False, "forbidden aggregate (sales_amount/gross_revenue/pay_amount)"
    if metric == "sales":
        if "biz_operating_amount" in s:
            return True, "uses biz_operating_amount"
        return False, "missing biz_operating_amount"
    if "ba_dws_pay_store_d" not in s:
        return False, "PR must use ba_dws_pay_store_d"
    if "refunded_amount" not in s:
        return False, "PR must subtract refunded_amount"
    if "pay_actual_amount" not in s:
        return False, "PR must use pay_actual_amount"
    return True, "PR formula ok"


def run_case(case: Case) -> dict[str, Any]:
    out: dict[str, Any] = {
        "lang": case.lang,
        "metric": case.metric,
        "prompt": case.prompt,
        "gateway": GATEWAY,
        "projId": PROJ_ID,
    }
    t0 = time.time()
    try:
        created = http_json(
            "POST",
            "/v1/solve_async",
            {
                "projId": PROJ_ID,
                "userPrompt": case.prompt,
                "timeoutSeconds": TIMEOUT_SEC,
                "extraSession": EXTRA,
            },
        )
        sid = created["sessionId"]
        tid = created.get("turnId")
        out["sessionId"] = sid
        out["turnId"] = tid

        deadline = t0 + TIMEOUT_SEC + 30
        final = None
        while time.time() < deadline:
            task = http_json("GET", f"/v1/tasks/{sid}?proj_id={PROJ_ID}")
            if task.get("status") in ("succeeded", "failed", "cancelled"):
                final = task
                break
            time.sleep(POLL_SEC)

        out["turnStatus"] = (final or {}).get("status", "timeout")
        if not tid:
            out["pass"] = False
            out["reason"] = "missing turnId"
            return out

        tools = http_json(
            "GET",
            f"/v1/sessions/{sid}/turns/{tid}/tools?proj_id={PROJ_ID}",
        )
        sql = extract_sql(tools)
        ok, reason = judge(case.metric, sql)
        out["sql"] = sql
        out["pass"] = ok and out["turnStatus"] == "succeeded"
        out["reason"] = reason
        if out["turnStatus"] != "succeeded":
            out["pass"] = False
            out["reason"] = f"turn {out['turnStatus']}; {reason}"
    except urllib.error.HTTPError as e:
        out["pass"] = False
        out["reason"] = f"HTTP {e.code}: {e.read().decode()[:300]}"
    except Exception as e:  # noqa: BLE001
        out["pass"] = False
        out["reason"] = str(e)
    out["wallSec"] = round(time.time() - t0, 1)
    return out


def main() -> int:
    results = [run_case(c) for c in CASES]
    passed = sum(1 for r in results if r.get("pass"))
    print(json.dumps(results, ensure_ascii=False, indent=2))
    print(f"\n=== {passed}/{len(results)} passed ===", file=sys.stderr)
    for r in results:
        mark = "PASS" if r.get("pass") else "FAIL"
        print(
            f"{mark} [{r['lang']}/{r['metric']}] {r.get('reason')} "
            f"({r.get('wallSec')}s session={r.get('sessionId', '-')})",
            file=sys.stderr,
        )
    return 0 if passed == len(results) else 1


if __name__ == "__main__":
    raise SystemExit(main())

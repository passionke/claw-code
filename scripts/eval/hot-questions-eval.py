#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""Evaluate hot-push questions against live gateway. Author: kejiqing"""

from __future__ import annotations

import json
import sys
import time
import urllib.error
import urllib.request
from dataclasses import dataclass
from typing import Any

GATEWAY = "http://10.200.2.171:18088"
PROJ_ID = 10
STORE = {
    "store_id": "S20241219215400003205",
    "store_name": "Bao&Beer",
    "org_id": "",
}
POLL_INTERVAL_S = 5
MAX_WAIT_S = 420


@dataclass(frozen=True)
class QuestionCase:
    qid: int
    lang: str
    text: str


QUESTIONS: list[QuestionCase] = [
    # Q1: last 7 days sales + how to improve
    QuestionCase(1, "zh", "近7天销售额多少，如何提升？"),
    QuestionCase(
        1,
        "en",
        "What were sales in the last 7 days, and how can I improve them?",
    ),
    QuestionCase(
        1,
        "th",
        "ยอดขาย 7 วันที่ผ่านมาเท่าไหร่ และจะเพิ่มยอดขายได้อย่างไร",
    ),
    QuestionCase(
        1,
        "my",
        "လွန်ခဲ့သော 7 ရက်ရောင်းအားဘယ်လောက်ရှိပါသလဲ၊ ဘယ်လိုတိုးမြှင့်နိုင်မလဲ။",
    ),
    # Q2: last 7 days best / slow-moving dishes
    QuestionCase(2, "zh", "近7天什么菜卖的好？什么菜滞销？"),
    QuestionCase(
        2,
        "en",
        "In the last 7 days, which dishes sell well? Which are slow-moving?",
    ),
    QuestionCase(
        2,
        "th",
        "7 วันที่ผ่านมา เมนูไหนขายดี เมนูไหนไม่ค่อยขาย",
    ),
    QuestionCase(
        2,
        "my",
        "လွန်ခဲ့သော 7 ရက်အတွင်း ဘယ်မီနူးတွေရောင်းအားကောင်းပြီး ဘယ်မီနူးတွေရောင်းအားမကောင်းတာလဲ။",
    ),
    # Q3: yesterday gross profit + margin
    QuestionCase(3, "zh", "昨天毛利和毛利率多少？"),
    QuestionCase(3, "en", "What was yesterday's gross profit and margin?"),
    QuestionCase(3, "th", "กำไรขั้นต้นและอัตรากำไรเมื่อวานเป็นเท่าไหร่"),
    QuestionCase(
        3,
        "my",
        "မနေ့က အသားတင်အမြတ်နဲ့ အမြတ်နှုန်းဘယ်လောက်ရှိပါသလဲ။",
    ),
]


def http_json(method: str, url: str, body: dict | None = None, timeout: int = 60) -> Any:
    data = None
    headers = {"Content-Type": "application/json", "Accept": "application/json"}
    if body is not None:
        data = json.dumps(body).encode("utf-8")
    req = urllib.request.Request(url, data=data, headers=headers, method=method)
    with urllib.request.urlopen(req, timeout=timeout) as resp:
        return json.loads(resp.read().decode("utf-8"))


def build_extra_session() -> dict[str, str]:
    return {
        "tenant_code": "GPOS",
        "solution_code": "restaurant",
        "biz_type": "BOSS_REPORT",
        "store_id": STORE["store_id"],
        "store_name": STORE["store_name"],
        "org_id": STORE["org_id"],
        "_claw_client_origin": "hot-question-eval",
    }


def submit_solve(user_prompt: str) -> dict[str, Any]:
    # New session: omit sessionId so gateway allocates one (explicit id requires PG history).
    body = {
        "projId": PROJ_ID,
        "userPrompt": user_prompt,
        "extraSession": build_extra_session(),
    }
    return http_json("POST", f"{GATEWAY}/v1/solve_async", body)


def poll_task(task_id: str) -> dict[str, Any]:
    deadline = time.time() + MAX_WAIT_S
    last: dict[str, Any] = {}
    while time.time() < deadline:
        last = http_json("GET", f"{GATEWAY}/v1/tasks/{task_id}", timeout=30)
        status = last.get("status")
        if status in ("succeeded", "failed", "cancelled"):
            return last
        time.sleep(POLL_INTERVAL_S)
    last["status"] = last.get("status", "timeout")
    return last


def extract_report(task: dict[str, Any]) -> str:
    result = task.get("result") or {}
    output = result.get("outputJson") or {}
    if isinstance(output, dict):
        msg = output.get("message")
        if isinstance(msg, str) and msg.strip():
            return msg.strip()
    err = task.get("error") or result.get("detail") or ""
    return str(err).strip()


def score_response(qid: int, lang: str, report: str, status: str) -> dict[str, Any]:
    r = report.lower()
    checks: dict[str, bool] = {
        "has_content": len(report.strip()) >= 80,
        "terminal_ok": status == "succeeded",
        "uses_data": any(
            x in r
            for x in [
                "ยอด",
                "ขาย",
                "sales",
                "revenue",
                "profit",
                "กำไร",
                "利润",
                "销售额",
                "รายได้",
                "ရောင်း",
                "အမြတ်",
                "%",
                "บาท",
                "thb",
            ]
        ),
        "lang_match": True,
        "actionable": any(
            x in r
            for x in [
                "แนะนำ",
                "建议",
                "recommend",
                "should",
                "ควร",
                "提升",
                "improve",
                "promot",
                "โปร",
                "တိုးမြှင့်",
            ]
        ),
    }
    if lang == "zh":
        checks["lang_match"] = any("\u4e00" <= c <= "\u9fff" for c in report) and not any(
            "\u0e00" <= c <= "\u0e7f" for c in report
        )
    elif lang == "th":
        checks["lang_match"] = any("\u0e00" <= c <= "\u0e7f" for c in report)
    elif lang == "my":
        checks["lang_match"] = any("\u1000" <= c <= "\u109f" for c in report)
    elif lang == "en":
        checks["lang_match"] = any(c.isalpha() for c in report) and not any(
            "\u4e00" <= c <= "\u9fff" or "\u0e00" <= c <= "\u0e7f" for c in report
        )

    if qid == 1:
        checks["answers_sales"] = checks["uses_data"]
        checks["answers_improve"] = checks["actionable"]
    elif qid == 2:
        checks["answers_best_slow"] = any(
            x in r
            for x in [
                "ขายดี",
                "best",
                "top",
                "滞销",
                "slow",
                "น้อย",
                "ไม่ค่อย",
                "ရောင်းအား",
            ]
        )
    elif qid == 3:
        checks["answers_gross_profit"] = any(
            x in r
            for x in [
                "กำไรขั้นต้น",
                "毛利",
                "gross profit",
                "အသားတင်အမြတ်",
                "净毛利",
            ]
        )
        checks["answers_margin_pct"] = "%" in report or any(
            x in r
            for x in ["margin", "毛利率", "อัตรากำไร", "အမြတ်နှုန်း", "71.", "70."]
        )

    passed = sum(1 for k, v in checks.items() if v)
    total = len(checks)
    return {"checks": checks, "score": f"{passed}/{total}"}


def main() -> int:
    results: list[dict[str, Any]] = []
    print(f"Gateway: {GATEWAY} projId={PROJ_ID} store={STORE['store_name']}", flush=True)
    for i, qc in enumerate(QUESTIONS, 1):
        label = f"Q{qc.qid}-{qc.lang}"
        print(f"\n[{i}/{len(QUESTIONS)}] {label}: {qc.text}", flush=True)
        try:
            enq = submit_solve(qc.text)
            task_id = enq.get("taskId") or enq.get("sessionId")
            turn_id = enq.get("turnId")
            print(f"  enqueued taskId={task_id} turnId={turn_id}", flush=True)
            task = poll_task(str(task_id))
            status = str(task.get("status"))
            report = extract_report(task)
            evaluation = score_response(qc.qid, qc.lang, report, status)
            preview = report[:500].replace("\n", " ")
            print(f"  status={status} score={evaluation['score']}", flush=True)
            print(f"  preview: {preview}...", flush=True)
            results.append(
                {
                    "qid": qc.qid,
                    "lang": qc.lang,
                    "question": qc.text,
                    "taskId": task_id,
                    "turnId": turn_id,
                    "status": status,
                    "report": report,
                    "evaluation": evaluation,
                    "progress": task.get("currentTaskDesc"),
                }
            )
        except Exception as e:
            print(f"  ERROR: {e}", flush=True)
            results.append(
                {
                    "qid": qc.qid,
                    "lang": qc.lang,
                    "question": qc.text,
                    "status": "error",
                    "error": str(e),
                }
            )

    out_path = "/tmp/hot_questions_eval.json"
    with open(out_path, "w", encoding="utf-8") as f:
        json.dump({"store": STORE, "results": results}, f, ensure_ascii=False, indent=2)
    print(f"\nWrote {out_path}", flush=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())

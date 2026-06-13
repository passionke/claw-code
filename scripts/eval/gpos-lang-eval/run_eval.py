#!/usr/bin/env python3
# GPOS language eval runner: solve_async + poll, concurrent sessions. Author: kejiqing
"""Run solve_async cases from questions.json against gateway proj 10."""

from __future__ import annotations

import argparse
import json
import os
import sys
import time
import urllib.error
import urllib.request
from concurrent.futures import ThreadPoolExecutor, as_completed
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parent
GATEWAY = os.environ.get("GATEWAY", "http://10.22.11.19:18088")
PROJ_ID = int(os.environ.get("PROJ_ID", "10"))
CONCURRENCY = int(os.environ.get("CONCURRENCY", "4"))
TIMEOUT_SEC = int(os.environ.get("TIMEOUT_SEC", "180"))
POLL_SEC = float(os.environ.get("POLL_SEC", "5"))
MAX_RETRIES = int(os.environ.get("MAX_RETRIES", "3"))


def http(method: str, path: str, body: dict | None = None, timeout: int = 60) -> Any:
    data = json.dumps(body, ensure_ascii=False).encode() if body is not None else None
    req = urllib.request.Request(
        f"{GATEWAY}{path}",
        data=data,
        headers={"Content-Type": "application/json"},
        method=method,
    )
    with urllib.request.urlopen(req, timeout=timeout) as resp:
        return json.loads(resp.read().decode())


def http_with_retry(method: str, path: str, body: dict | None = None, timeout: int = 60) -> Any:
    delay = 2.0
    last_err: Exception | None = None
    for attempt in range(MAX_RETRIES):
        try:
            return http(method, path, body, timeout=timeout)
        except urllib.error.HTTPError as e:
            last_err = e
            if e.code in (503, 502, 429, 500) and attempt + 1 < MAX_RETRIES:
                time.sleep(delay)
                delay *= 2
                continue
            raise
        except (urllib.error.URLError, TimeoutError) as e:
            last_err = e
            if attempt + 1 < MAX_RETRIES:
                time.sleep(delay)
                delay *= 2
                continue
            raise
    raise last_err  # type: ignore[misc]


def load_questions(path: Path) -> dict:
    with path.open(encoding="utf-8") as f:
        return json.load(f)


def expand_cases(questions_doc: dict, langs: list[str] | None = None) -> list[dict]:
    langs = langs or ["zh", "en", "th"]
    extra = questions_doc["extraSession"]
    cases = []
    for q in questions_doc["questions"]:
        for lang in langs:
            prompt = q[lang]
            case_id = f"{q['id']}_{lang}"
            cases.append(
                {
                    "caseId": case_id,
                    "questionId": q["id"],
                    "category": q["category"],
                    "lang": lang,
                    "userPrompt": prompt,
                    "extraSession": extra,
                    "notes": q.get("notes"),
                }
            )
    return cases


def run_one_case(case: dict, cases_dir: Path) -> dict:
    case_id = case["caseId"]
    out_path = cases_dir / f"{case_id}.json"
    if out_path.is_file():
        with out_path.open(encoding="utf-8") as f:
            existing = json.load(f)
        if existing.get("status") in ("succeeded", "failed", "cancelled"):
            print(f"[skip] {case_id} already done ({existing.get('status')})", flush=True)
            return existing

    print(f"[start] {case_id} prompt={case['userPrompt'][:40]!r}", flush=True)
    start = time.time()
    result: dict[str, Any] = {
        "caseId": case_id,
        "questionId": case["questionId"],
        "category": case["category"],
        "lang": case["lang"],
        "userPrompt": case["userPrompt"],
        "gateway": GATEWAY,
        "projId": PROJ_ID,
    }

    try:
        created = http_with_retry(
            "POST",
            "/v1/solve_async",
            {
                "projId": PROJ_ID,
                "userPrompt": case["userPrompt"],
                "timeoutSeconds": TIMEOUT_SEC,
                "extraSession": case["extraSession"],
            },
            timeout=60,
        )
        session_id = created["sessionId"]
        turn_id = created.get("turnId")
        result["sessionId"] = session_id
        result["turnId"] = turn_id

        deadline = start + TIMEOUT_SEC + 30
        final = None
        while time.time() < deadline:
            task = http_with_retry("GET", f"/v1/tasks/{session_id}?proj_id={PROJ_ID}", timeout=60)
            status = task.get("status")
            if status in ("succeeded", "failed", "cancelled"):
                final = task
                break
            time.sleep(POLL_SEC)

        result["wallSec"] = round(time.time() - start, 1)
        if final is None:
            result["status"] = "timeout"
            result["error"] = f"no terminal status within {TIMEOUT_SEC}s"
        else:
            result["status"] = final.get("status")
            result["planTitle"] = final.get("planTitle")
            result["currentTaskDesc"] = final.get("currentTaskDesc")
            result["progressHistory"] = final.get("progressHistory") or []
            result["hasReport"] = final.get("hasReport")
            if final.get("error"):
                result["error"] = final["error"]
            if final.get("status") == "succeeded" and final.get("result"):
                out_json = final["result"].get("outputJson") or {}
                result["message"] = out_json.get("message", "")

            try:
                exec_data = http_with_retry(
                    "GET",
                    f"/v1/sessions/{session_id}/execution?proj_id={PROJ_ID}",
                    timeout=30,
                )
                prog = exec_data.get("progress") or {}
                result["progressDesc"] = prog.get("currentTaskDesc") or result.get("currentTaskDesc")
                result["progressPlanTitle"] = prog.get("planTitle") or result.get("planTitle")
            except Exception as e:  # noqa: BLE001
                result["executionFetchError"] = str(e)

    except Exception as e:  # noqa: BLE001
        result["wallSec"] = round(time.time() - start, 1)
        result["status"] = "error"
        result["error"] = str(e)

    with out_path.open("w", encoding="utf-8") as f:
        json.dump(result, f, ensure_ascii=False, indent=2)

    print(
        f"[done] {case_id} status={result.get('status')} wall={result.get('wallSec')}s "
        f"session={result.get('sessionId', '-')}",
        flush=True,
    )
    return result


def main() -> int:
    parser = argparse.ArgumentParser(description="GPOS lang eval runner")
    parser.add_argument("--questions", type=Path, default=ROOT / "questions.json")
    parser.add_argument("--cases-dir", type=Path, default=ROOT / "cases")
    parser.add_argument("--resume", action="store_true", help="skip cases with terminal status")
    parser.add_argument("--case", action="append", dest="only_cases", help="run specific case e.g. Q01_zh")
    parser.add_argument("--lang", action="append", dest="only_langs", choices=["zh", "en", "th"])
    parser.add_argument("--concurrency", type=int, default=CONCURRENCY)
    args = parser.parse_args()

    args.cases_dir.mkdir(parents=True, exist_ok=True)
    doc = load_questions(args.questions)
    cases = expand_cases(doc, args.only_langs)

    if args.only_cases:
        allowed = set(args.only_cases)
        cases = [c for c in cases if c["caseId"] in allowed]

    if args.resume:
        pending = []
        for c in cases:
            p = args.cases_dir / f"{c['caseId']}.json"
            if p.is_file():
                with p.open(encoding="utf-8") as f:
                    prev = json.load(f)
                if prev.get("status") in ("succeeded", "failed", "cancelled", "timeout", "error"):
                    continue
            pending.append(c)
        cases = pending

    if not cases:
        print("no cases to run", flush=True)
        return 0

    print(
        f"gateway={GATEWAY} proj={PROJ_ID} cases={len(cases)} concurrency={args.concurrency}",
        flush=True,
    )

    results = []
    with ThreadPoolExecutor(max_workers=args.concurrency) as pool:
        futures = {pool.submit(run_one_case, c, args.cases_dir): c for c in cases}
        for fut in as_completed(futures):
            try:
                results.append(fut.result())
            except Exception as e:  # noqa: BLE001
                c = futures[fut]
                print(f"[fatal] {c['caseId']}: {e}", file=sys.stderr, flush=True)
                results.append({"caseId": c["caseId"], "status": "error", "error": str(e)})

    ok = sum(1 for r in results if r.get("status") == "succeeded")
    print(f"finished: {ok}/{len(results)} succeeded", flush=True)
    return 0 if ok == len(results) else 1


if __name__ == "__main__":
    sys.exit(main())

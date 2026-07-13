#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""Build ≥100 bilingual core questions + live gateway_solve batch for proj 271.

Thai questions → expect gpos.co.th/th/... ; others → /en/...
Author: kejiqing
"""

from __future__ import annotations

import argparse
import json
import os
import re
import sys
import time
import urllib.request
from collections import Counter
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
ROOT = Path(os.environ.get("GPOS_MANUAL_KB", REPO / "knowledge" / "gpos-user-manual"))
EVAL = Path(os.environ.get("GPOS_MANUAL_EVAL_OUT", ROOT / "eval"))
MCP_URL = os.environ.get("CLAW_ADMIN_MCP_URL", "http://192.168.9.252:18088/v1/admin/mcp")
TOKEN = os.environ.get("CLAW_ADMIN_TOKEN", "").strip()
EXTRA = {
    "store_id": "S002501221841976200006188",
    "store_name": "G&G Ratchaburi",
    "org_id": "",
}


def parse_frontmatter(text: str) -> dict[str, str]:
    if not text.startswith("---"):
        return {}
    end = text.find("\n---", 3)
    if end < 0:
        return {}
    meta: dict[str, str] = {}
    for line in text[3:end].splitlines():
        if ":" not in line:
            continue
        k, v = line.split(":", 1)
        meta[k.strip()] = v.strip().strip('"')
    return meta


def collect_docs(lang: str) -> list[dict]:
    base = ROOT / lang
    docs = []
    for md in sorted(base.rglob("*.md")):
        rel = md.relative_to(ROOT).as_posix()
        if md.name in {"index.md", "README.md"}:
            continue
        text = md.read_text(encoding="utf-8")
        meta = parse_frontmatter(text)
        url = meta.get("source_url") or ""
        if f"/{lang}/user-manual" not in url:
            continue
        title = meta.get("title") or md.stem
        body = text[text.find("\n---", 3) + 4 :] if text.startswith("---") else text
        if len(re.sub(r"\s+", " ", body)) < 80:
            continue
        docs.append(
            {
                "title": title,
                "category": meta.get("category") or md.parent.name,
                "path": rel,
                "source_url": url,
                "lang_kb": lang,
                "body_len": len(body),
            }
        )
    return docs


def build_questions(min_n: int = 100) -> list[dict]:
    en_docs = sorted(collect_docs("en"), key=lambda d: -d["body_len"])
    th_docs = sorted(collect_docs("th"), key=lambda d: -d["body_len"])
    rows: list[dict] = []

    def add(doc: dict, qlang: str, question: str, i: int) -> None:
        kb = "th" if qlang == "th" else "en"
        if doc["lang_kb"] != kb:
            return
        rows.append(
            {
                "id": f"{doc['path'].replace('/', '-').replace('.md', '')}-{qlang}-{i:02d}",
                "category": doc["category"],
                "question": question,
                "lang": qlang,
                "expected_source_url": doc["source_url"],
                "expected_doc_path": doc["path"],
                "expected_url_lang": kb,
                "must_include": [x for x in re.findall(r"[A-Za-z\u0E00-\u0E7F]{4,}", doc["title"])[:3]]
                or [doc["title"][:12]],
                "must_not_call_mcp": True,
                "intent": "product_manual",
            }
        )

    # bilingual membership pair first (always kept)
    for lang, q in [
        ("en", "How do I add a member in Back Office?"),
        ("zh", "后台怎么新增会员？"),
        ("th", "เพิ่มสมาชิกในระบบหลังบ้านอย่างไร?"),
    ]:
        kb = "th" if lang == "th" else "en"
        path = f"{kb}/membership/add-member-back-office.md"
        p = ROOT / path
        if p.exists():
            meta = parse_frontmatter(p.read_text(encoding="utf-8"))
            rows.append(
                {
                    "id": f"pair-add-member-{lang}",
                    "category": "Membership",
                    "question": q,
                    "lang": lang,
                    "expected_source_url": meta.get("source_url"),
                    "expected_doc_path": path,
                    "expected_url_lang": kb,
                    "must_include": ["member"] if lang != "th" else ["สมาชิก"],
                    "must_not_call_mcp": True,
                    "intent": "product_manual",
                }
            )

    n_each = 34
    for i, d in enumerate(en_docs[:n_each]):
        add(d, "en", f"How do I set up or complete: {d['title']}?", i + 1)
    for i, d in enumerate(en_docs[:n_each]):
        add(d, "zh", f"GPOS 里「{d['title']}」怎么操作？", i + 1)
    for i, d in enumerate(th_docs[:n_each]):
        add(d, "th", f"ใน GPOS ตั้งค่าหรือทำ「{d['title']}」อย่างไร?", i + 1)

    seen = set()
    out = []
    for r in rows:
        if r["id"] in seen:
            continue
        seen.add(r["id"])
        out.append(r)
    if len(out) < min_n:
        raise SystemExit(f"only {len(out)} questions")
    return out[: max(min_n, len(out) if len(out) < 120 else 105)]


def mcp_solve(prompt: str, timeout: int = 180) -> dict:
    payload = {
        "jsonrpc": "2.0",
        "id": int(time.time() * 1000) % 10**9,
        "method": "tools/call",
        "params": {
            "name": "gateway_solve",
            "arguments": {
                "projId": 271,
                "userPrompt": prompt,
                "extraSession": EXTRA,
                "timeoutSeconds": timeout,
            },
        },
    }
    req = urllib.request.Request(
        MCP_URL,
        data=json.dumps(payload).encode(),
        headers={
            "Authorization": f"Bearer {TOKEN}",
            "Content-Type": "application/json",
            "Accept": "application/json, text/event-stream",
        },
        method="POST",
    )
    with urllib.request.urlopen(req, timeout=timeout + 90) as resp:
        body = json.loads(resp.read().decode())
    if body.get("error"):
        raise RuntimeError(str(body["error"]))
    text = ""
    for c in (body.get("result") or {}).get("content") or []:
        if isinstance(c, dict) and c.get("type") == "text":
            text += c.get("text") or ""
    return json.loads(text)


def extract_message(solve: dict) -> str:
    oj = solve.get("outputJson") or {}
    if isinstance(oj, str):
        try:
            oj = json.loads(oj)
        except json.JSONDecodeError:
            oj = {}
    if isinstance(oj, dict) and oj.get("message"):
        return str(oj["message"])
    ot = solve.get("outputText") or ""
    try:
        return str(json.loads(ot).get("message") or ot)
    except Exception:
        return ot


def score(row: dict, message: str, solve: dict) -> dict:
    urls = re.findall(r"https?://gpos\.co\.th/(en|th)/user-manual[^\s)\]\"'<>]*", message)
    expected = row["expected_source_url"]
    exp_lang = row["expected_url_lang"]
    url_exact = expected.rstrip("/") in message.replace(")", " ")
    url_lang_ok = any(u == exp_lang for u in urls)
    wrong_lang = any(u != exp_lang for u in urls)
    must = row.get("must_include") or []
    low = message.lower()
    must_hits = sum(1 for m in must if m.lower() in low)
    must_need = max(1, int(0.5 * max(1, len(must))))
    intro = ("restaurant operations assistant" in low or "经营助手" in low) and not urls
    ok = bool(urls) and url_lang_ok and not wrong_lang and not intro and must_hits >= must_need
    return {
        "id": row["id"],
        "lang": row["lang"],
        "category": row["category"],
        "pass": ok,
        "url_exact": url_exact,
        "url_lang_ok": url_lang_ok,
        "wrong_lang_url": wrong_lang,
        "found_url_langs": list(urls),
        "must_hits": must_hits,
        "must_total": len(must),
        "looks_intro": intro,
        "sessionId": solve.get("sessionId"),
        "durationMs": solve.get("durationMs"),
        "clawExitCode": solve.get("clawExitCode"),
        "message_preview": message[:350],
        "question": row["question"],
        "expected_source_url": expected,
    }


def write_jsonl(path: Path, rows: list[dict]) -> None:
    path.write_text("".join(json.dumps(r, ensure_ascii=False) + "\n" for r in rows), encoding="utf-8")


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--build-only", action="store_true")
    ap.add_argument("--limit", type=int, default=0, help="limit live cases (0=all)")
    ap.add_argument("--min", type=int, default=100)
    args = ap.parse_args()

    rows = build_questions(args.min)
    write_jsonl(EVAL / "core-questions.jsonl", rows)
    print("built", len(rows), Counter(r["lang"] for r in rows))
    if args.build_only:
        return 0
    if not TOKEN:
        print("缺少环境变量 CLAW_ADMIN_TOKEN", file=sys.stderr)
        return 2

    cases = rows if not args.limit else rows[: args.limit]
    results = []
    for i, row in enumerate(cases, 1):
        print(f"[{i}/{len(cases)}] {row['id']} ...", flush=True)
        t0 = time.time()
        try:
            solve = mcp_solve(row["question"], timeout=180)
            msg = extract_message(solve)
            r = score(row, msg, solve)
            r["elapsedSec"] = round(time.time() - t0, 1)
            r["error"] = None
        except Exception as e:
            r = {
                "id": row["id"],
                "lang": row["lang"],
                "pass": False,
                "error": str(e),
                "elapsedSec": round(time.time() - t0, 1),
                "question": row["question"],
                "expected_source_url": row["expected_source_url"],
            }
        print(
            json.dumps(
                {k: r.get(k) for k in ("id", "pass", "url_lang_ok", "wrong_lang_url", "error", "elapsedSec")},
                ensure_ascii=False,
            ),
            flush=True,
        )
        results.append(r)
        time.sleep(0.4)

    write_jsonl(EVAL / "results.jsonl", results)
    passed = sum(1 for r in results if r.get("pass"))
    by_lang = Counter(r["lang"] for r in results)
    pass_by_lang = Counter(r["lang"] for r in results if r.get("pass"))
    url_lang_ok = sum(1 for r in results if r.get("url_lang_ok"))
    wrong = sum(1 for r in results if r.get("wrong_lang_url"))
    summary = {
        "total": len(results),
        "passed": passed,
        "pass_rate": round(passed / max(1, len(results)), 4),
        "url_lang_ok_rate": round(url_lang_ok / max(1, len(results)), 4),
        "wrong_lang_url_count": wrong,
        "by_lang": dict(by_lang),
        "pass_by_lang": dict(pass_by_lang),
        "failed_ids": [r["id"] for r in results if not r.get("pass")],
        "contentRev": "2026-07-13_05-49-00",
        "projId": 271,
    }
    fail_ids = summary["failed_ids"][:80]
    fail_block = [f"- `{i}`" for i in fail_ids] if fail_ids else ["_none_"]
    (EVAL / "summary.md").write_text(
        "\n".join(
            [
                "# Live product-manual eval summary (proj 271)",
                "",
                f"- total: **{summary['total']}**",
                f"- passed: **{summary['passed']}** ({summary['pass_rate']:.1%})",
                f"- url_lang_ok_rate: **{summary['url_lang_ok_rate']:.1%}**",
                f"- wrong_lang_url_count: **{summary['wrong_lang_url_count']}**",
                f"- by_lang: `{summary['by_lang']}`",
                f"- pass_by_lang: `{summary['pass_by_lang']}`",
                f"- contentRev: `{summary['contentRev']}`",
                "",
                "## Failed ids",
                "",
                *fail_block,
                "",
                "Author: kejiqing",
                "",
            ]
        ),
        encoding="utf-8",
    )
    (EVAL / "summary.json").write_text(json.dumps(summary, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    fails = [r for r in results if not r.get("pass")]
    fail_lines = ["# Live failures", ""]
    for f in fails:
        fail_lines.append(
            f"- `{f.get('id')}` lang={f.get('lang')} err={f.get('error')} urls={f.get('found_url_langs')} preview={(f.get('message_preview') or '')[:120]}"
        )
    if not fails:
        fail_lines.append("_none_")
    (EVAL / "failures.md").write_text("\n".join(fail_lines) + "\n", encoding="utf-8")
    print("SUMMARY", json.dumps(summary, ensure_ascii=False))
    return 0 if summary["pass_rate"] >= 0.9 and summary["wrong_lang_url_count"] == 0 else 1


if __name__ == "__main__":
    raise SystemExit(main())

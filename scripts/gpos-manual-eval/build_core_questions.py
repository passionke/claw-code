#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""Build ≥100 core product-manual questions from crawled KB + offline verify.

Author: kejiqing
"""

from __future__ import annotations

import argparse
import json
import os
import re
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
# Local crawl output only — not committed (see .gitignore knowledge/)
ROOT = Path(os.environ.get("GPOS_MANUAL_KB", REPO / "knowledge" / "gpos-user-manual"))
EVAL = Path(os.environ.get("GPOS_MANUAL_EVAL_OUT", ROOT / "eval"))

SKIP_REL = {"index.md", "getting-started.md", "README.md", "manifest.json"}


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


def body_after_frontmatter(text: str) -> str:
    if not text.startswith("---"):
        return text
    end = text.find("\n---", 3)
    if end < 0:
        return text
    return text[end + 4 :]


def extract_must_include(title: str, body: str) -> list[str]:
    cues: list[str] = []
    # login / menu paths
    for m in re.finditer(
        r"(login\.gpos\.co\.th|Item\s*>\s*Item|Back Office|POS|GrabFood|Head Office)",
        body,
        flags=re.I,
    ):
        cues.append(m.group(1))
    # numbered step verbs
    for m in re.finditer(r"(?m)^\s*\d+[\.\)]\s+(.{12,80})", body):
        frag = re.sub(r"\s+", " ", m.group(1)).strip()
        # take 2-4 content words
        words = re.findall(r"[A-Za-z][A-Za-z0-9-]{2,}", frag)
        if words:
            cues.append(words[0])
        if len(words) >= 2:
            cues.append(words[1])
        if len(cues) >= 6:
            break
    # title tokens
    for w in re.findall(r"[A-Za-z][A-Za-z0-9-]{3,}", title):
        if w.lower() not in {"back", "office", "with", "from", "that", "this", "guide"}:
            cues.append(w)
        if len(cues) >= 8:
            break
    # unique preserve order, lower for matching later we keep original
    out: list[str] = []
    seen: set[str] = set()
    for c in cues:
        key = c.lower()
        if key in seen:
            continue
        seen.add(key)
        out.append(c)
        if len(out) >= 4:
            break
    while len(out) < 2 and title:
        # fallback
        parts = [p for p in re.split(r"\W+", title) if len(p) >= 3]
        for p in parts:
            if p.lower() not in seen:
                out.append(p)
                seen.add(p.lower())
            if len(out) >= 2:
                break
        break
    return out[:4]


def questions_for(doc: dict, lang: str) -> list[dict]:
    title = doc["title"]
    cat = doc["category"]
    path = doc["path"]
    url = doc["source_url"]
    must = doc["must_include"]
    base_id = path.replace("/", "-").replace(".md", "").replace("index", "hub")

    templates = {
        "en": [
            f"How do I set up or complete: {title}?",
            f"Steps for {title} in GPOS",
        ],
        "zh": [
            f"GPOS 里「{title}」怎么操作？",
            f"请给出「{title}」的后台/POS 步骤",
        ],
        "th": [
            f"ใน GPOS ตั้งค่าหรือทำ「{title}」อย่างไร?",
            f"ขอขั้นตอนสำหรับ「{title}」",
        ],
    }
    qs = []
    for i, q in enumerate(templates[lang][:1]):
        qs.append(
            {
                "id": f"{base_id}-{lang}-{i+1:02d}",
                "category": cat,
                "question": q,
                "lang": lang,
                "expected_source_url": url,
                "expected_doc_path": path,
                "must_include": must,
                "must_not_call_mcp": True,
                "intent": "product_manual",
            }
        )
    return qs


def collect_docs(kb_root: Path) -> list[dict]:
    docs: list[dict] = []
    for md in sorted(kb_root.rglob("*.md")):
        rel = md.relative_to(kb_root).as_posix()
        if rel in SKIP_REL or rel.startswith("eval/"):
            continue
        if md.name == "README.md":
            continue
        text = md.read_text(encoding="utf-8")
        meta = parse_frontmatter(text)
        body = body_after_frontmatter(text)
        title = meta.get("title") or md.stem.replace("-", " ").title()
        url = meta.get("source_url") or ""
        cat = meta.get("category") or md.parent.name
        if not url:
            continue
        # skip empty media stubs for primary set unless we still need volume
        body_plain = re.sub(r"\s+", " ", body).strip()
        must = extract_must_include(title, body)
        docs.append(
            {
                "title": title,
                "category": cat,
                "path": rel,
                "source_url": url,
                "must_include": must,
                "body_len": len(body_plain),
            }
        )
    return docs


def build_core(docs: list[dict], min_n: int = 100) -> list[dict]:
    # Prefer articles with real body
    ranked = sorted(docs, key=lambda d: (-d["body_len"], d["path"]))
    rows: list[dict] = []
    # Round-robin langs for diversity
    langs = ["en", "zh", "th"]
    # First pass: one question per article with body_len>=120
    for i, d in enumerate(ranked):
        if d["body_len"] < 80:
            continue
        lang = langs[i % 3]
        rows.extend(questions_for(d, lang))
        if len(rows) >= min_n:
            return rows[: min_n + 20]  # slight buffer then trim
    # Second pass: thin docs + extra templates
    for i, d in enumerate(ranked):
        if len(rows) >= min_n:
            break
        lang = langs[i % 3]
        for q in questions_for(d, lang):
            if q["id"] in {r["id"] for r in rows}:
                # use second template variant
                q = questions_for(d, langs[(i + 1) % 3])[0]
                q["id"] = q["id"] + "-x"
            rows.append(q)
            if len(rows) >= min_n:
                break
    # Third pass: add 2nd lang for top articles to reach 100+
    extra = 0
    for i, d in enumerate(ranked):
        if len(rows) >= min_n:
            break
        if d["body_len"] < 120:
            continue
        lang = langs[(i + 1) % 3]
        for q in questions_for(d, lang):
            q["id"] = q["id"].replace(f"-{lang}-", f"-{lang}b-")
            if any(r["id"] == q["id"] for r in rows):
                continue
            rows.append(q)
            extra += 1
            if len(rows) >= min_n:
                break
    return rows


def write_jsonl(path: Path, rows: list[dict]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("w", encoding="utf-8") as f:
        for r in rows:
            f.write(json.dumps(r, ensure_ascii=False) + "\n")


def write_contrast_sets() -> None:
    chitchat = [
        {
            "id": f"chat-{i:02d}",
            "question": q,
            "lang": lang,
            "intent": "chitchat",
            "must_not_call_mcp": True,
            "expected_skill": "self-introduction",
        }
        for i, (lang, q) in enumerate(
            [
                ("en", "Hi, who are you?"),
                ("en", "Tell me a joke"),
                ("en", "What's the weather in Bangkok?"),
                ("zh", "你好，你能做什么？"),
                ("zh", "讲个笑话吧"),
                ("zh", "帮我写一段 Python 代码"),
                ("th", "สวัสดี คุณคือใคร"),
                ("th", "เล่าเรื่องตลกให้ฟังหน่อย"),
                ("en", "What model are you?"),
                ("zh", "今天股市怎么样？"),
            ],
            start=1,
        )
    ]
    analysis = [
        {
            "id": f"an-{i:02d}",
            "question": q,
            "lang": lang,
            "intent": "analysis",
            "must_not_use_manual_kb": True,
            "expect_sqlbot": True,
        }
        for i, (lang, q) in enumerate(
            [
                ("en", "What were yesterday's sales?"),
                ("en", "Show payment method breakdown for last week."),
                ("en", "Which menu items had the highest revenue this month?"),
                ("zh", "昨天的销售额和订单量是多少？"),
                ("zh", "最近 7 天各收款方式占比？"),
                ("zh", "本月销量最高的菜品有哪些？"),
                ("th", "ยอดขายเมื่อวานเป็นเท่าไร?"),
                ("th", "สัดส่วนช่องทางชำระเงิน 7 วันที่ผ่านมา?"),
                ("en", "Compare this week's performance to last week."),
                ("zh", "对比上周，本周营业额变化如何？"),
            ],
            start=1,
        )
    ]
    write_jsonl(EVAL / "chitchat.jsonl", chitchat)
    write_jsonl(EVAL / "analysis.jsonl", analysis)


def verify_offline(rows: list[dict], kb_root: Path) -> tuple[list[dict], dict]:
    results = []
    url_ok = 0
    must_ok = 0
    path_ok = 0
    for r in rows:
        doc_path = kb_root / r["expected_doc_path"]
        entry = {
            "id": r["id"],
            "pass": False,
            "path_exists": doc_path.is_file(),
            "url_in_doc": False,
            "must_hit": 0,
            "must_total": len(r.get("must_include") or []),
            "fail_reasons": [],
        }
        if not entry["path_exists"]:
            entry["fail_reasons"].append("missing_doc")
            results.append(entry)
            continue
        path_ok += 1
        text = doc_path.read_text(encoding="utf-8")
        url = r["expected_source_url"]
        entry["url_in_doc"] = url in text
        if entry["url_in_doc"]:
            url_ok += 1
        else:
            entry["fail_reasons"].append("url_missing")
        hits = 0
        low = text.lower()
        for cue in r.get("must_include") or []:
            if cue.lower() in low:
                hits += 1
        entry["must_hit"] = hits
        need = max(1, int(0.8 * max(1, entry["must_total"])))
        if hits >= need:
            must_ok += 1
        else:
            entry["fail_reasons"].append("must_include")
        entry["pass"] = not entry["fail_reasons"]
        results.append(entry)
    n = max(1, len(rows))
    summary = {
        "total": len(rows),
        "path_exists_rate": path_ok / n,
        "source_url_hit_rate": url_ok / n,
        "must_include_pass_rate": must_ok / n,
        "pass_rate": sum(1 for x in results if x["pass"]) / n,
    }
    return results, summary


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--min", type=int, default=100)
    ap.add_argument("--kb", type=Path, default=ROOT)
    args = ap.parse_args()
    docs = collect_docs(args.kb)
    rows = build_core(docs, min_n=args.min)
    if len(rows) < args.min:
        raise SystemExit(f"only built {len(rows)} questions, need >={args.min}")
    # trim to at least min, keep extras up to min+15 for buffer then cut to exactly >=100
    if len(rows) > args.min + 30:
        rows = rows[: args.min + 10]
    write_jsonl(EVAL / "core-questions.jsonl", rows)
    write_contrast_sets()
    results, summary = verify_offline(rows, args.kb)
    write_jsonl(EVAL / "results-offline.jsonl", results)
    (EVAL / "summary-offline.md").write_text(
        "\n".join(
            [
                "# Offline KB eval summary",
                "",
                f"- total: {summary['total']}",
                f"- path_exists_rate: {summary['path_exists_rate']:.1%}",
                f"- source_url_hit_rate: {summary['source_url_hit_rate']:.1%}",
                f"- must_include_pass_rate: {summary['must_include_pass_rate']:.1%}",
                f"- pass_rate: {summary['pass_rate']:.1%}",
                "",
                "Author: kejiqing",
                "",
            ]
        ),
        encoding="utf-8",
    )
    fails = [r for r in results if not r["pass"]]
    fail_lines = ["# Offline failures", ""]
    for f in fails[:50]:
        fail_lines.append(f"- `{f['id']}`: {', '.join(f['fail_reasons'])}")
    if not fails:
        fail_lines.append("_none_")
    fail_lines.append("")
    (EVAL / "failures-offline.md").write_text("\n".join(fail_lines), encoding="utf-8")
    print(json.dumps(summary, indent=2))
    print(f"wrote {len(rows)} questions -> {EVAL / 'core-questions.jsonl'}")
    return 0 if summary["pass_rate"] >= 0.9 and summary["source_url_hit_rate"] >= 0.95 else 1


if __name__ == "__main__":
    raise SystemExit(main())

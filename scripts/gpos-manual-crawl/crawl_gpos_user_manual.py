#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""Crawl GPOS user manual (EN + TH) into local knowledge/gpos-user-manual/{en,th}/.

Output is gitignored — business KB is not stored in claw-code.
Raw extract only — no LLM rewrite. Author: kejiqing
"""

from __future__ import annotations

import argparse
import html as html_lib
import json
import os
import re
import time
import urllib.error
import urllib.parse
import urllib.request
from collections import deque
from dataclasses import dataclass, field
from datetime import datetime, timezone
from html.parser import HTMLParser
from pathlib import Path

BASE = "https://gpos.co.th"
UA = "Mozilla/5.0 (compatible; claw-gpos-kb-crawl/1.0; +https://github.com/passionke/claw-code)"
SUPPORTED_LANGS = ("en", "th")

CATEGORY_SLUGS = {
    "system-setup-program-usage": "System Setup & Program Usage",
    "product-management": "Product Management",
    "warehouse-management": "Warehouse Management",
    "manager-app": "Manager App",
    "discounts": "Discounts",
    "staff-management": "Staff Management",
    "membership": "Membership",
    "printer-settings": "Printer Settings",
    "head-office": "Head Office",
    "device-settings": "Device Settings",
    "store-management-back-office": "Store Management (Back Office)",
    "grab-x-gpos": "Grab X Gpos",
    "gpos-pro": "Gpos Pro",
    "faq": "FAQ",
}

# Thread-local-ish for parser link filter
_CURRENT_LANG = "en"


def seed_for(lang: str) -> str:
    return f"{BASE}/{lang}/user-manual"


def fetch(url: str, timeout: float = 45.0) -> str:
    req = urllib.request.Request(url, headers={"User-Agent": UA, "Accept": "text/html"})
    with urllib.request.urlopen(req, timeout=timeout) as resp:
        raw = resp.read()
        charset = resp.headers.get_content_charset() or "utf-8"
    return raw.decode(charset, errors="replace")


def normalize_url(href: str, lang: str | None = None) -> str | None:
    lang = lang or _CURRENT_LANG
    if not href or href.startswith(("#", "mailto:", "tel:", "javascript:")):
        return None
    abs_url = urllib.parse.urljoin(BASE, href)
    parsed = urllib.parse.urlparse(abs_url)
    if parsed.netloc and parsed.netloc != "gpos.co.th":
        return None
    path = parsed.path.rstrip("/") or "/"
    prefix = f"/{lang}/user-manual"
    if not path.startswith(prefix):
        return None
    return f"{BASE}{path}"


def path_parts(url: str, lang: str) -> list[str]:
    path = urllib.parse.urlparse(url).path
    parts = [p for p in path.split("/") if p]
    if len(parts) >= 2 and parts[0] == lang and parts[1] == "user-manual":
        return parts[2:]
    return []


def category_for(url: str, lang: str) -> tuple[str, str]:
    parts = path_parts(url, lang)
    if not parts:
        return "root", "Getting Started"
    slug = parts[0]
    return slug, CATEGORY_SLUGS.get(slug, slug.replace("-", " ").title())


def rel_md_path(url: str, lang: str) -> str:
    parts = path_parts(url, lang)
    if not parts:
        return "getting-started.md"
    if len(parts) == 1:
        return f"{parts[0]}/index.md"
    return f"{parts[0]}/{'/'.join(parts[1:])}.md"


class ManualHTMLParser(HTMLParser):
    SKIP_TAGS = {
        "script",
        "style",
        "noscript",
        "svg",
        "nav",
        "footer",
        "header",
        "iframe",
    }

    def __init__(self, lang: str) -> None:
        super().__init__(convert_charrefs=True)
        self.lang = lang
        self.title = ""
        self.links: set[str] = set()
        self._in_title = False
        self._skip_depth = 0
        self._article_depth = 0
        self._capture = False
        self._buf: list[str] = []
        self._tag_stack: list[str] = []
        self.blocks: list[str] = []
        self.h1 = ""

    def handle_starttag(self, tag: str, attrs: list[tuple[str, str | None]]) -> None:
        ad = {k: (v or "") for k, v in attrs}
        if tag in self.SKIP_TAGS:
            self._skip_depth += 1
            return
        if self._skip_depth:
            return
        if tag == "title":
            self._in_title = True
        if tag == "a":
            nu = normalize_url(ad.get("href", ""), self.lang)
            if nu:
                self.links.add(nu)
        if tag == "article":
            self._article_depth += 1
            self._capture = True
        cls = ad.get("class", "")
        if tag == "main" or (tag == "div" and "font-ttcommons" in cls and self._article_depth == 0):
            if not self._capture:
                self._capture = True
        if self._capture and tag in {"p", "h1", "h2", "h3", "h4", "li", "br"}:
            if tag == "br":
                self._buf.append("\n")
            elif tag == "li":
                self._buf.append("\n- ")
            elif tag.startswith("h"):
                if self._buf:
                    self._flush_block()
                level = int(tag[1])
                self._buf.append("#" * level + " ")
        self._tag_stack.append(tag)

    def handle_endtag(self, tag: str) -> None:
        if tag in self.SKIP_TAGS:
            if self._skip_depth:
                self._skip_depth -= 1
            return
        if self._skip_depth:
            return
        if tag == "title":
            self._in_title = False
        if tag == "article" and self._article_depth:
            self._article_depth -= 1
            if self._article_depth == 0:
                self._flush_block()
        if self._capture and tag in {"p", "h1", "h2", "h3", "h4", "li", "div", "section"}:
            self._flush_block()
        if self._tag_stack and self._tag_stack[-1] == tag:
            self._tag_stack.pop()
        elif tag in self._tag_stack:
            while self._tag_stack and self._tag_stack[-1] != tag:
                self._tag_stack.pop()
            if self._tag_stack:
                self._tag_stack.pop()

    def handle_data(self, data: str) -> None:
        if self._skip_depth:
            return
        if self._in_title and data.strip():
            self.title += data.strip() + " "
            return
        if not self._capture:
            return
        if not data:
            return
        self._buf.append(data)

    def _flush_block(self) -> None:
        raw = "".join(self._buf)
        self._buf.clear()
        cleaned = re.sub(r"[ \t]+", " ", raw)
        cleaned = re.sub(r"\n{3,}", "\n\n", cleaned).strip()
        if not cleaned or len(cleaned) < 2:
            return
        if cleaned.startswith("# ") and not self.h1:
            self.h1 = cleaned.lstrip("# ").strip()
        self.blocks.append(cleaned)


NAV_CATEGORY_NAMES = {
    "system setup & program usage",
    "product management",
    "warehouse management",
    "manager app",
    "discounts",
    "staff management",
    "membership",
    "printer settings",
    "head office",
    "device settings",
    "store management (back office)",
    "grab x gpos",
    "gpos pro",
    "getting started",
    "user manual categories",
    "contact us",
    "frequently asked questions",
    "see more",
    "หัวข้อ",
    "how to",
    "computer",
    "circle",
    "user manual",
    "of",
    "logo",
    "เริ่มต้นระบบ & การใช้งานโปรแกรม",
    "การจัดการรายการสินค้า",
    "การจัดการคลังสินค้า",
    "การจัดการส่วนลด",
    "การจัดการพนักงาน",
    "ระบบสมาชิก",
    "การจัดการเครื่องพิมพ์",
    "ระบบสำนักงานใหญ่",
    "การตั้งค่า/อื่นๆ (หน้าเครื่อง)",
    "การจัดการการตั้งค่าทั้งหมด (ระบบหลังบ้าน)",
}


def is_chrome_block(text: str) -> bool:
    key = re.sub(r"\s+", " ", text).strip().lower()
    if not key:
        return True
    if key in {x.lower() for x in NAV_CATEGORY_NAMES}:
        return True
    if key.startswith("gpos an assistant"):
        return True
    if re.fullmatch(r"#?\s*\d{4}-\d{2}-\d{2}([ t]\d{1,2}:\d{2})?", key):
        return True
    if re.fullmatch(r"[\d,]+\s*(views?|ผู้ชม)", key):
        return True
    if re.fullmatch(r"\d+\s*(คู่มือ|manuals?|articles?)", key):
        return True
    return False


def strip_leading_nav(blocks: list[str]) -> list[str]:
    start = 0
    for i, b in enumerate(blocks):
        if is_chrome_block(b):
            start = i + 1
            continue
        plain = re.sub(r"^#+\s*", "", b).strip()
        if re.match(r"^\d+[\.\)]\s+", plain) or len(plain) >= 60:
            start = i
            break
        start = i + 1
    return blocks[start:] if start < len(blocks) else blocks


def dedupe_blocks(blocks: list[str]) -> list[str]:
    out: list[str] = []
    seen: set[str] = set()
    for b in blocks:
        if is_chrome_block(b):
            continue
        key = re.sub(r"\s+", " ", b).strip().lower()
        if key in seen:
            continue
        seen.add(key)
        out.append(b)
    return strip_leading_nav(out)


def regex_fallback_blocks(html: str) -> list[str]:
    texts = re.findall(
        r"<p[^>]*class=\"[^\"]*font-ttcommons[^\"]*\"[^>]*>(.*?)</p>",
        html,
        flags=re.I | re.S,
    )
    blocks: list[str] = []
    for t in texts:
        plain = re.sub(r"<br\s*/?>", "\n", t, flags=re.I)
        plain = re.sub(r"<[^>]+>", "", plain)
        plain = html_lib.unescape(plain)
        plain = re.sub(r"[ \t]+", " ", plain)
        plain = re.sub(r"\n{3,}", "\n\n", plain).strip()
        if len(plain) >= 20:
            blocks.append(plain)
    return dedupe_blocks(blocks)


def extract_page(url: str, html: str, lang: str) -> tuple[str, set[str], list[str]]:
    parser = ManualHTMLParser(lang)
    parser.feed(html)
    title = (parser.h1 or parser.title or url).strip()
    title = re.sub(r"\s*-\s*Gpos\s*$", "", title, flags=re.I).strip()
    title = html_lib.unescape(title)
    blocks = dedupe_blocks(parser.blocks)
    if len("\n".join(blocks)) < 80:
        blocks = regex_fallback_blocks(html)
    return title, parser.links, blocks


def keywords_from(title: str, body: str) -> list[str]:
    words = re.findall(r"[A-Za-z\u0E00-\u0E7F][A-Za-z0-9\u0E00-\u0E7F-]{2,}", title + " " + body[:500])
    stop = {"the", "and", "for", "with", "from", "this", "that", "your", "into", "then", "click", "gpos", "how", "via"}
    out: list[str] = []
    seen: set[str] = set()
    for w in words:
        lw = w.lower()
        if lw in stop or lw in seen:
            continue
        seen.add(lw)
        out.append(w)
        if len(out) >= 12:
            break
    return out


def yaml_escape(s: str) -> str:
    if re.search(r'[:#"\'\n]', s):
        return '"' + s.replace("\\", "\\\\").replace('"', '\\"') + '"'
    return s


def render_md(
    *,
    title: str,
    source_url: str,
    lang: str,
    category: str,
    category_slug: str,
    keywords: list[str],
    body_blocks: list[str],
    crawled_at: str,
) -> str:
    lines = [
        "---",
        f"title: {yaml_escape(title)}",
        f"source_url: {source_url}",
        f"lang: {lang}",
        f"category: {yaml_escape(category)}",
        f"category_slug: {category_slug}",
        f"keywords: [{', '.join(yaml_escape(k) for k in keywords)}]",
        f"crawled_at: {crawled_at}",
        "---",
        "",
        f"# {title}",
        "",
        f"**Official docs:** {source_url}",
        "",
    ]
    if body_blocks:
        lines.append("## Steps / Content")
        lines.append("")
        for b in body_blocks:
            lines.append(b)
            lines.append("")
    else:
        lines.append("_No extractable body text; open the official docs URL._")
        lines.append("")
    if keywords:
        lines.append("## Keywords")
        lines.append("")
        lines.append(", ".join(keywords))
        lines.append("")
    lines.append(f"<!-- Author: kejiqing; lang={lang}; crawled_at={crawled_at} -->")
    lines.append("")
    return "\n".join(lines)


@dataclass
class PageDoc:
    url: str
    title: str
    lang: str
    category_slug: str
    category: str
    rel_path: str
    body_len: int
    keywords: list[str] = field(default_factory=list)


def crawl_lang(out_root: Path, lang: str, delay_s: float, max_pages: int | None) -> list[PageDoc]:
    global _CURRENT_LANG
    _CURRENT_LANG = lang
    out_dir = out_root / lang
    out_dir.mkdir(parents=True, exist_ok=True)
    crawled_at = datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
    seed = seed_for(lang)
    q: deque[str] = deque([seed])
    seen: set[str] = set()
    docs: list[PageDoc] = []

    while q:
        if max_pages is not None and len(docs) >= max_pages:
            break
        url = q.popleft()
        if url in seen:
            continue
        seen.add(url)
        try:
            html = fetch(url)
        except (urllib.error.URLError, TimeoutError, OSError) as e:
            print(f"FAIL fetch {url}: {e}")
            continue
        title, links, blocks = extract_page(url, html, lang)
        cat_slug, cat_name = category_for(url, lang)
        rel = rel_md_path(url, lang)
        if url.rstrip("/") == seed.rstrip("/"):
            rel = "getting-started.md"
            cat_slug, cat_name = "root", "Getting Started"
        keywords = keywords_from(title, "\n".join(blocks))
        md = render_md(
            title=title,
            source_url=url,
            lang=lang,
            category=cat_name,
            category_slug=cat_slug,
            keywords=keywords,
            body_blocks=blocks,
            crawled_at=crawled_at,
        )
        dest = out_dir / rel
        dest.parent.mkdir(parents=True, exist_ok=True)
        dest.write_text(md, encoding="utf-8")
        docs.append(
            PageDoc(
                url=url,
                title=title,
                lang=lang,
                category_slug=cat_slug,
                category=cat_name,
                rel_path=f"{lang}/{rel}",
                body_len=sum(len(b) for b in blocks),
                keywords=keywords,
            )
        )
        print(f"OK [{lang}] {rel} ({docs[-1].body_len} chars) <- {url}")
        for link in sorted(links):
            if link not in seen:
                q.append(link)
        if delay_s > 0:
            time.sleep(delay_s)

    write_lang_index(out_dir, docs, crawled_at, lang, seed)
    manifest = {
        "lang": lang,
        "source": seed,
        "crawled_at": crawled_at,
        "page_count": len(docs),
        "pages": [
            {
                "title": d.title,
                "source_url": d.url,
                "lang": lang,
                "category": d.category,
                "category_slug": d.category_slug,
                "path": d.rel_path,
                "body_len": d.body_len,
            }
            for d in docs
        ],
    }
    (out_dir / "manifest.json").write_text(
        json.dumps(manifest, ensure_ascii=False, indent=2) + "\n", encoding="utf-8"
    )
    return docs


def write_lang_index(out_dir: Path, docs: list[PageDoc], crawled_at: str, lang: str, seed: str) -> None:
    by_cat: dict[str, list[PageDoc]] = {}
    for d in docs:
        by_cat.setdefault(d.category, []).append(d)
    lines = [
        "---",
        f'title: "GPOS User Manual Index ({lang})"',
        f"source_url: {seed}",
        f"lang: {lang}",
        'category: "Index"',
        "category_slug: index",
        f"crawled_at: {crawled_at}",
        "---",
        "",
        f"# GPOS User Manual Index ({lang})",
        "",
        f"Source: {seed}",
        "",
        f"Pages: **{len(docs)}** · crawled `{crawled_at}`",
        "",
        f"Runtime path: `/claw_ds/home/kb/{lang}/`",
        "",
    ]
    for cat in sorted(by_cat.keys(), key=lambda c: c.lower()):
        lines.append(f"## {cat}")
        lines.append("")
        for d in sorted(by_cat[cat], key=lambda x: x.rel_path):
            rel = d.rel_path.split("/", 1)[1] if d.rel_path.startswith(f"{lang}/") else d.rel_path
            lines.append(f"- [{d.title}]({rel}) — {d.url}")
        lines.append("")
    lines.append("<!-- Author: kejiqing -->")
    lines.append("")
    (out_dir / "index.md").write_text("\n".join(lines), encoding="utf-8")


def write_root_index(out_root: Path, all_docs: list[PageDoc], crawled_at: str) -> None:
    en_n = sum(1 for d in all_docs if d.lang == "en")
    th_n = sum(1 for d in all_docs if d.lang == "th")
    text = f"""---
title: "GPOS User Manual KB Root"
source_url: https://gpos.co.th/en/user-manual
category: Index
category_slug: index
crawled_at: {crawled_at}
---

# GPOS User Manual KB

Author: kejiqing

Raw crawl only (no LLM rewrite). Language routing:

- User Thai input → search `/claw_ds/home/kb/th/` and use `source_url` under `https://gpos.co.th/th/user-manual/...`
- Otherwise → search `/claw_ds/home/kb/en/` and use `https://gpos.co.th/en/user-manual/...`

## Indexes

- [English index](en/index.md) — {en_n} pages — https://gpos.co.th/en/user-manual
- [Thai index](th/index.md) — {th_n} pages — https://gpos.co.th/th/user-manual

Crawled at `{crawled_at}`.
"""
    (out_root / "index.md").write_text(text, encoding="utf-8")
    (out_root / "manifest.json").write_text(
        json.dumps(
            {
                "crawled_at": crawled_at,
                "languages": ["en", "th"],
                "page_count": len(all_docs),
                "en_count": en_n,
                "th_count": th_n,
            },
            ensure_ascii=False,
            indent=2,
        )
        + "\n",
        encoding="utf-8",
    )


def main() -> int:
    ap = argparse.ArgumentParser(description="Crawl GPOS EN/TH user manuals")
    ap.add_argument(
        "--out",
        type=Path,
        default=Path(
            os.environ.get(
                "GPOS_MANUAL_KB",
                str(Path(__file__).resolve().parents[2] / "knowledge" / "gpos-user-manual"),
            )
        ),
    )
    ap.add_argument("--lang", choices=["en", "th", "all"], default="all")
    ap.add_argument("--delay", type=float, default=0.2)
    ap.add_argument("--max-pages", type=int, default=None)
    args = ap.parse_args()
    langs = list(SUPPORTED_LANGS) if args.lang == "all" else [args.lang]
    # clear old flat layout leftovers when doing full bilingual crawl
    if args.lang == "all":
        for name in list(args.out.iterdir()) if args.out.exists() else []:
            if name.name in {"en", "th", "eval", "README.md", "index.md", "manifest.json"}:
                continue
            # remove legacy flat category dirs
            if name.is_dir() or name.suffix in {".md", ".json"}:
                import shutil

                if name.is_dir():
                    shutil.rmtree(name)
                else:
                    name.unlink()
    all_docs: list[PageDoc] = []
    for lang in langs:
        docs = crawl_lang(args.out, lang, args.delay, args.max_pages)
        all_docs.extend(docs)
        print(f"Lang {lang}: {len(docs)} pages")
    crawled_at = datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
    write_root_index(args.out, all_docs, crawled_at)
    print(f"Done total={len(all_docs)} -> {args.out}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

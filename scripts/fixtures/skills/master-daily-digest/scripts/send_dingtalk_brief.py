#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""Send signed DingTalk custom-robot markdown brief.

Credentials from env only:
  Webhook  — full robot webhook URL (with access_token)
  security — DingTalk custom-robot sign secret (SEC…)

Author: kejiqing
"""
from __future__ import annotations

import argparse
import base64
import hashlib
import hmac
import json
import os
import time
import urllib.error
import urllib.parse
import urllib.request

FOLDER_URL = "https://alidocs.dingtalk.com/i/nodes/gvNG4YZ7JneLjaZOs96YyobaV2LD0oRE"


def load_robot_from_env() -> tuple[str, str]:
    webhook = (os.environ.get("Webhook") or "").strip()
    secret = (os.environ.get("security") or "").strip()
    if not webhook:
        raise SystemExit("missing env Webhook (DingTalk robot webhook URL)")
    if not secret:
        raise SystemExit("missing env security (DingTalk robot sign secret)")
    return webhook, secret


def signed_url(webhook: str, secret: str) -> str:
    timestamp = str(round(time.time() * 1000))
    string_to_sign = f"{timestamp}\n{secret}"
    digest = hmac.new(
        secret.encode("utf-8"),
        string_to_sign.encode("utf-8"),
        digestmod=hashlib.sha256,
    ).digest()
    sign = urllib.parse.quote_plus(base64.b64encode(digest))
    sep = "&" if "?" in webhook else "?"
    return f"{webhook}{sep}timestamp={timestamp}&sign={sign}"


def build_text(args: argparse.Namespace) -> str:
    highlights = [h.strip() for h in (args.highlights or "").split("|") if h.strip()]
    lines = [
        f"### Master日报简报 · 学徒{args.apprentice_id} · {args.bizdate}",
        "",
        f"- 会话/轮次：**{args.sessions}/{args.turns}**（前日 {args.prev_sessions}/{args.prev_turns}）",
    ]
    if args.satisfy_rate:
        lines.append(f"- 需求满足率：**{args.satisfy_rate}**")
    if args.revisit_rate:
        lines.append(f"- store 近30天复访率：**{args.revisit_rate}**")
    if args.avg_latency or args.p90_latency:
        lines.append(
            f"- 轮次耗时：均 **{args.avg_latency or 'n/a'}** / P90 **{args.p90_latency or 'n/a'}**"
        )
    if highlights:
        lines.append("- 要点：")
        for h in highlights[:5]:
            lines.append(f"  - {h}")
    if args.doc_url:
        lines.append("")
        lines.append(f"[打开完整钉钉文档]({args.doc_url})")
    lines.append("")
    lines.append(f"目录：[clawcode-output]({FOLDER_URL})")
    return "\n".join(lines)


def post_markdown(url: str, title: str, text: str) -> dict:
    body = json.dumps(
        {"msgtype": "markdown", "markdown": {"title": title, "text": text}},
        ensure_ascii=False,
    ).encode("utf-8")
    req = urllib.request.Request(
        url,
        data=body,
        method="POST",
        headers={"Content-Type": "application/json; charset=utf-8"},
    )
    with urllib.request.urlopen(req, timeout=30) as resp:
        return json.loads(resp.read().decode("utf-8"))


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--title", required=True)
    ap.add_argument("--doc-url", default="")
    ap.add_argument("--bizdate", required=True)
    ap.add_argument("--apprentice-id", type=int, required=True)
    ap.add_argument("--sessions", required=True)
    ap.add_argument("--turns", required=True)
    ap.add_argument("--prev-sessions", default="?")
    ap.add_argument("--prev-turns", default="?")
    ap.add_argument("--satisfy-rate", default="")
    ap.add_argument("--revisit-rate", default="")
    ap.add_argument("--avg-latency", default="")
    ap.add_argument("--p90-latency", default="")
    ap.add_argument("--highlights", default="")
    args = ap.parse_args()

    webhook, secret = load_robot_from_env()
    text = build_text(args)
    url = signed_url(webhook, secret)
    try:
        result = post_markdown(url, args.title, text)
    except urllib.error.HTTPError as e:
        raw = e.read().decode("utf-8", errors="replace")
        print(json.dumps({"ok": False, "http": e.code, "body": raw}, ensure_ascii=False))
        return 1
    ok = result.get("errcode") == 0
    print(json.dumps({"ok": ok, "result": result, "preview": text[:500]}, ensure_ascii=False))
    return 0 if ok else 2


if __name__ == "__main__":
    raise SystemExit(main())

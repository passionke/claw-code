#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""Send CJK repair proposal brief to DingTalk (hardcoded webhook + sign).

Author: kejiqing

Only call after promote_to_apprentice_draft succeeded.
"""
from __future__ import annotations

import argparse
import base64
import hashlib
import hmac
import json
import time
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path

# Hardcoded per product request for CJK proposal notifications. Author: kejiqing
WEBHOOK = (
    "https://oapi.dingtalk.com/robot/send"
    "?access_token=78ef2286c1096072d23a4852dd712c7393361d3d8c2258b9b0117671f3f9008a"
)
SECRET = "SEC2f35cf58cd3d68154fe26fd2058480202c68f29242c901600796ab8869097d39"


def signed_url(webhook: str, secret: str) -> str:
    ts = str(round(time.time() * 1000))
    digest = hmac.new(
        secret.encode("utf-8"),
        f"{ts}\n{secret}".encode("utf-8"),
        digestmod=hashlib.sha256,
    ).digest()
    sign = urllib.parse.quote_plus(base64.b64encode(digest))
    sep = "&" if "?" in webhook else "?"
    return f"{webhook}{sep}timestamp={ts}&sign={sign}"


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--bizdate", required=True)
    ap.add_argument("--apprentice-id", type=int, required=True)
    ap.add_argument("--score-json", required=True)
    ap.add_argument("--run-id", default="")
    args = ap.parse_args()

    score = json.loads(Path(args.score_json).read_text(encoding="utf-8"))
    if not score.get("promote_recommended"):
        print(json.dumps({"ok": False, "skipped": "promote_recommended=false"}, ensure_ascii=False))
        return 2

    cjk = score.get("cjk") or {}
    ctrl = score.get("control") or {}
    text = "\n".join(
        [
            f"### CJK泰文修复提案 · 学徒{args.apprentice_id} · {args.bizdate}",
            "",
            "- 状态：**已推送到学徒草稿**（未 activate，请人工 review）",
            f"- repair run: `{args.run_id or 'n/a'}`",
            f"- 病例: n={cjk.get('n')} CJK {cjk.get('baselineCjkHits')}→{cjk.get('afterCjkHits')}",
            f"- 对照: n={ctrl.get('n')} afterCJK={ctrl.get('afterCjkHits')} "
            f"empty={ctrl.get('emptyAfter')} fail={ctrl.get('failedAfter')}",
            f"- mitigated={score.get('mitigated')} controlOk={score.get('controlOk')}",
            "",
            "请在 Admin 打开学徒草稿 diff，确认后再 activate。",
        ]
    )
    body = json.dumps(
        {
            "msgtype": "markdown",
            "markdown": {
                "title": f"CJK修复提案 · 学徒{args.apprentice_id} · {args.bizdate}",
                "text": text,
            },
        },
        ensure_ascii=False,
    ).encode("utf-8")
    url = signed_url(WEBHOOK, SECRET)
    req = urllib.request.Request(
        url,
        data=body,
        method="POST",
        headers={"Content-Type": "application/json; charset=utf-8"},
    )
    try:
        with urllib.request.urlopen(req, timeout=30) as resp:
            result = json.loads(resp.read().decode("utf-8"))
    except urllib.error.HTTPError as e:
        raw = e.read().decode("utf-8", errors="replace")
        print(json.dumps({"ok": False, "http": e.code, "body": raw}, ensure_ascii=False))
        return 1
    ok = result.get("errcode") == 0
    print(json.dumps({"ok": ok, "result": result}, ensure_ascii=False))
    return 0 if ok else 2


if __name__ == "__main__":
    raise SystemExit(main())

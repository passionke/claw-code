#!/usr/bin/env python3
"""Pull a full gateway session (turns + per-turn tools/timeline) for QA.

Author: kejiqing

Gateway 环境（勿混用）:
  - 生产 ONLY_READ: http://10.200.2.171:18088（本脚本默认；与 alfred admin/chat 同后端）
  - 预发: http://192.168.9.252:18088（写 skill/config、验收 solve 用此地址）
  - 本地: http://10.22.11.19:18088

本脚本仅 GET session 数据，ONLY_READ。禁止对默认生产地址做 POST/PUT（skill、config、术语等）。
"""

from __future__ import annotations

import argparse
import json
import os
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parent
# 生产 gateway，ONLY_READ。仅拉会话诊断；写操作走预发 192.168.9.252:18088。
DEFAULT_GATEWAY = os.environ.get("GATEWAY", "http://10.200.2.171:18088")
DEFAULT_PROJ_ID = int(os.environ.get("PROJ_ID", "10"))


def http_json(method: str, url: str, timeout: int = 60) -> Any:
    req = urllib.request.Request(url, method=method)
    with urllib.request.urlopen(req, timeout=timeout) as resp:
        return json.loads(resp.read().decode())


def fetch_turn_extras(
    gateway: str,
    session_id: str,
    turn_id: str,
    proj_id: int,
    *,
    include_timeline: bool,
) -> dict[str, Any]:
    base = f"{gateway}/v1/sessions/{session_id}/turns/{turn_id}"
    q = f"proj_id={proj_id}"
    out: dict[str, Any] = {}
    try:
        out["tools"] = http_json("GET", f"{base}/tools?{q}")
    except urllib.error.HTTPError as e:
        out["toolsError"] = f"HTTP {e.code}"
    if include_timeline:
        try:
            out["timeline"] = http_json("GET", f"{base}/timeline?{q}")
        except urllib.error.HTTPError as e:
            out["timelineError"] = f"HTTP {e.code}"
    return out


def fetch_session(
    gateway: str,
    session_id: str,
    proj_id: int,
    *,
    include_timeline: bool = False,
) -> dict[str, Any]:
    turns_url = (
        f"{gateway}/v1/sessions/{session_id}/turns?proj_id={proj_id}"
    )
    bundle = http_json("GET", turns_url)
    turns = bundle.get("turns") or []
    enriched = []
    for t in turns:
        turn_id = t.get("turnId") or ""
        extra = fetch_turn_extras(
            gateway, session_id, turn_id, proj_id, include_timeline=include_timeline
        )
        enriched.append({**t, **extra})
        time.sleep(0.05)
    return {
        "fetchedAtMs": int(time.time() * 1000),
        "gateway": gateway,
        "sessionId": session_id,
        "projId": proj_id,
        "turns": enriched,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--session-id", required=True)
    parser.add_argument("--proj-id", type=int, default=DEFAULT_PROJ_ID)
    parser.add_argument(
        "--gateway",
        default=DEFAULT_GATEWAY,
        help="生产 ONLY_READ 默认 10.200.2.171:18088；本脚本仅 GET，不写 skill/config",
    )
    parser.add_argument(
        "--out-dir",
        type=Path,
        default=None,
        help="default: scripts/conversation-qa/cases/<session_id>",
    )
    parser.add_argument("--timeline", action="store_true")
    args = parser.parse_args()

    out_dir = args.out_dir or (ROOT / "cases" / args.session_id)
    out_dir.mkdir(parents=True, exist_ok=True)

    try:
        data = fetch_session(
            args.gateway.rstrip("/"),
            args.session_id,
            args.proj_id,
            include_timeline=args.timeline,
        )
    except urllib.error.URLError as e:
        print(f"gateway unreachable: {e}", file=sys.stderr)
        return 1
    except urllib.error.HTTPError as e:
        print(f"HTTP {e.code}: {e.read().decode()[:500]}", file=sys.stderr)
        return 1

    out_path = out_dir / "session.json"
    with out_path.open("w", encoding="utf-8") as f:
        json.dump(data, f, ensure_ascii=False, indent=2)

    n = len(data.get("turns") or [])
    print(f"wrote {out_path} ({n} turns)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

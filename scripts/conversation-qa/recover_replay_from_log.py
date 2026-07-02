#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""Recover partial replay JSON from terminal log + dev gateway GET.

Author: kejiqing

When replay_manifest.py is interrupted, completed sessions can be rebuilt from
log lines `[N/88] <prodSessionId> ... devSession=<id>` plus manifest + dev turns.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parent
DEFAULT_DEV_GATEWAY = "http://10.22.28.94:18088"
DEFAULT_PROD_GATEWAY = "http://10.200.2.171:18088"
LOG_SESSION = re.compile(r"^\[(\d+)/\d+\]\s+([0-9a-f]{32})\s+\((\d+)\s+turns\)")
LOG_DONE = re.compile(r"done status=(\w+)\s+devSession=([0-9a-f]{32})")


def http_json(url: str, timeout: int = 60) -> Any:
    req = urllib.request.Request(url, headers={"Accept": "application/json"})
    with urllib.request.urlopen(req, timeout=timeout) as resp:
        return json.loads(resp.read().decode())


def parse_log(path: Path) -> list[dict[str, str]]:
    rows: list[dict[str, str]] = []
    pending: dict[str, str] | None = None
    for line in path.read_text(encoding="utf-8", errors="replace").splitlines():
        m_sess = LOG_SESSION.search(line.strip())
        if m_sess:
            pending = {
                "index": m_sess.group(1),
                "prodSessionId": m_sess.group(2),
                "turnCount": m_sess.group(3),
            }
            continue
        if pending:
            m_done = LOG_DONE.search(line.strip())
            if m_done:
                rows.append(
                    {
                        **pending,
                        "devStatus": m_done.group(1),
                        "devSessionId": m_done.group(2),
                    }
                )
                pending = None
    return rows


def fetch_turns(gateway: str, proj_id: int, session_id: str) -> list[dict[str, Any]]:
    url = f"{gateway.rstrip('/')}/v1/sessions/{session_id}/turns?proj_id={proj_id}"
    return http_json(url).get("turns") or []


def build_extra_session(item: dict[str, Any]) -> dict[str, str]:
    es = dict(item.get("extraSession") or {})
    es.setdefault("store_id", item.get("storeId") or "")
    es.setdefault("store_name", item.get("storeName") or "")
    es.setdefault("org_id", item.get("orgId") or "")
    es["bizdate"] = str(item.get("bizdate") or "")
    return {k: str(v) for k, v in es.items()}


def recover(
    log_rows: list[dict[str, str]],
    manifest: dict[str, Any],
    *,
    dev_gateway: str,
    prod_gateway: str,
    dev_proj_id: int,
    prod_proj_id: int,
) -> dict[str, Any]:
    by_session: dict[str, list[dict[str, Any]]] = {}
    for item in manifest.get("items") or []:
        by_session.setdefault(str(item["sessionId"]), []).append(item)
    for sid in by_session:
        by_session[sid].sort(key=lambda x: int(x["turnIndex"]))

    results: list[dict[str, Any]] = []
    for row in log_rows:
        prod_sid = row["prodSessionId"]
        dev_sid = row["devSessionId"]
        manifest_turns = by_session.get(prod_sid) or []
        try:
            prod_turns = fetch_turns(prod_gateway, prod_proj_id, prod_sid)
            dev_turns = fetch_turns(dev_gateway, dev_proj_id, dev_sid)
        except urllib.error.HTTPError as e:
            results.append(
                {
                    "prodSessionId": prod_sid,
                    "devSessionId": dev_sid,
                    "devStatus": "fetch_error",
                    "error": f"HTTP {e.code}",
                }
            )
            continue

        prod_by_prompt = {str(t.get("userPrompt") or ""): t for t in prod_turns}
        dev_by_idx = sorted(dev_turns, key=lambda t: int(t.get("createdAtMs") or 0))
        for i, item in enumerate(manifest_turns, 1):
            prompt = str(item.get("userPrompt") or "")
            prod_t = prod_by_prompt.get(prompt) or (
                prod_turns[i - 1] if i <= len(prod_turns) else {}
            )
            dev_t = dev_by_idx[i - 1] if i <= len(dev_by_idx) else {}
            results.append(
                {
                    "prodSessionId": prod_sid,
                    "prodTurnId": item.get("turnId") or prod_t.get("turnId"),
                    "turnIndex": item.get("turnIndex"),
                    "turnCount": item.get("turnCount"),
                    "bizdate": item.get("bizdate"),
                    "questionAt": item.get("questionAt"),
                    "storeId": item.get("storeId"),
                    "storeName": item.get("storeName"),
                    "orgId": item.get("orgId"),
                    "extraSession": build_extra_session(item),
                    "userPrompt": prompt,
                    "prodStatus": str(prod_t.get("status") or item.get("status") or ""),
                    "prodReport": str(prod_t.get("reportBody") or ""),
                    "devSessionId": dev_sid,
                    "devTurnId": dev_t.get("turnId"),
                    "devStatus": str(dev_t.get("status") or row["devStatus"]),
                    "devReport": str(dev_t.get("reportBody") or ""),
                }
            )
        time.sleep(0.05)

    return {
        "recoveredAtMs": int(time.time() * 1000),
        "source": "recover_replay_from_log",
        "logSessions": len(log_rows),
        "devGateway": dev_gateway,
        "prodGateway": prod_gateway,
        "devProjId": dev_proj_id,
        "prodProjId": prod_proj_id,
        "sessionCount": len(log_rows),
        "turnCount": len(results),
        "results": results,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--log",
        type=Path,
        default=Path(
            "/Users/sm4645/.cursor/projects/Users-sm4645-work-claw-code/terminals/12.txt"
        ),
    )
    parser.add_argument(
        "--manifest",
        type=Path,
        default=ROOT / "cases" / "proj10_manifest_20260614_20260615.json",
    )
    parser.add_argument("--dev-gateway", default=DEFAULT_DEV_GATEWAY)
    parser.add_argument("--prod-gateway", default=DEFAULT_PROD_GATEWAY)
    parser.add_argument("--dev-proj-id", type=int, default=27)
    parser.add_argument("--prod-proj-id", type=int, default=10)
    parser.add_argument(
        "--out",
        type=Path,
        default=ROOT / "cases" / "proj10_partial25_replay_p27.json",
    )
    args = parser.parse_args()

    log_rows = parse_log(args.log)
    if not log_rows:
        print(f"no completed sessions in log: {args.log}", file=sys.stderr)
        return 1

    with args.manifest.open(encoding="utf-8") as f:
        manifest = json.load(f)

    payload = recover(
        log_rows,
        manifest,
        dev_gateway=args.dev_gateway.rstrip("/"),
        prod_gateway=args.prod_gateway.rstrip("/"),
        dev_proj_id=args.dev_proj_id,
        prod_proj_id=args.prod_proj_id,
    )
    args.out.parent.mkdir(parents=True, exist_ok=True)
    with args.out.open("w", encoding="utf-8") as f:
        json.dump(payload, f, ensure_ascii=False, indent=2)

    ok = sum(1 for r in payload["results"] if r.get("devStatus") == "succeeded")
    print(f"wrote {args.out}")
    print(f"sessions={payload['sessionCount']} turns={payload['turnCount']} dev_ok={ok}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

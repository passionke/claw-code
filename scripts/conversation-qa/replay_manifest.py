#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""Replay prod manifest on dev gateway (proj 27) with bizdate + session continuation.

Author: kejiqing

Maps prod proj 10 conversations to dev proj 27 (simplified prompt stack).
Preserves multi-turn order: each session reuses dev sessionId from prior turn.
"""

from __future__ import annotations

import argparse
import json
import os
import sys
import time
import urllib.error
import urllib.request
from collections import defaultdict
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parent
DEFAULT_DEV_GATEWAY = os.environ.get("DEV_GATEWAY", "http://10.22.28.94:18088").rstrip("/")
DEFAULT_DEV_PROJ_ID = int(os.environ.get("DEV_PROJ_ID", "27"))
POLL_SEC = float(os.environ.get("POLL_SEC", "5"))
TIMEOUT_SEC = int(os.environ.get("TIMEOUT_SEC", "420"))


def http_json(
    gateway: str,
    method: str,
    path: str,
    body: dict | None = None,
    timeout: int = 60,
) -> Any:
    data = json.dumps(body, ensure_ascii=False).encode() if body is not None else None
    req = urllib.request.Request(
        f"{gateway}{path}",
        data=data,
        headers={"Content-Type": "application/json", "Accept": "application/json"},
        method=method,
    )
    with urllib.request.urlopen(req, timeout=timeout) as resp:
        return json.loads(resp.read().decode())


def build_extra_session(item: dict[str, Any]) -> dict[str, str]:
    es = dict(item.get("extraSession") or {})
    es.setdefault("store_id", item.get("storeId") or "")
    es.setdefault("store_name", item.get("storeName") or "")
    es.setdefault("org_id", item.get("orgId") or "")
    es["bizdate"] = str(item.get("bizdate") or "")
    return {k: str(v) for k, v in es.items()}


def submit_solve(
    gateway: str,
    proj_id: int,
    user_prompt: str,
    extra_session: dict[str, str],
    session_id: str | None,
) -> dict[str, Any]:
    body: dict[str, Any] = {
        "projId": proj_id,
        "userPrompt": user_prompt,
        "extraSession": extra_session,
        "_claw_client_origin": "conversation-qa-replay",
    }
    if session_id:
        body["sessionId"] = session_id
    return http_json(gateway, "POST", "/v1/solve_async", body)


def poll_task(gateway: str, task_id: str) -> dict[str, Any]:
    deadline = time.time() + TIMEOUT_SEC
    last: dict[str, Any] = {}
    while time.time() < deadline:
        last = http_json(gateway, "GET", f"/v1/tasks/{task_id}", timeout=30)
        status = last.get("status")
        if status in ("succeeded", "failed", "cancelled"):
            return last
        time.sleep(POLL_SEC)
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


def group_by_session(items: list[dict[str, Any]]) -> list[tuple[str, list[dict[str, Any]]]]:
    buckets: dict[str, list[dict[str, Any]]] = defaultdict(list)
    for item in items:
        buckets[str(item["sessionId"])].append(item)
    groups = []
    for sid, turns in buckets.items():
        turns.sort(key=lambda x: int(x["turnIndex"]))
        groups.append((sid, turns))
    groups.sort(key=lambda g: int(g[1][0]["questionAtMs"]))
    return groups


def replay_session(
    gateway: str,
    dev_proj_id: int,
    prod_session_id: str,
    turns: list[dict[str, Any]],
) -> list[dict[str, Any]]:
    dev_session_id: str | None = None
    results: list[dict[str, Any]] = []
    for item in turns:
        extra = build_extra_session(item)
        enq = submit_solve(
            gateway,
            dev_proj_id,
            str(item["userPrompt"]),
            extra,
            dev_session_id,
        )
        task_id = str(enq.get("taskId") or enq.get("sessionId") or "")
        task = poll_task(gateway, task_id)
        status = str(task.get("status"))
        report = extract_report(task)
        result = task.get("result") or {}
        dev_session_id = str(
            result.get("sessionId") or enq.get("sessionId") or dev_session_id or ""
        ) or None
        dev_turn_id = str(result.get("turnId") or enq.get("turnId") or "")
        results.append(
            {
                "prodSessionId": prod_session_id,
                "prodTurnId": item.get("turnId"),
                "turnIndex": item.get("turnIndex"),
                "turnCount": item.get("turnCount"),
                "bizdate": item.get("bizdate"),
                "questionAt": item.get("questionAt"),
                "storeId": item.get("storeId"),
                "storeName": item.get("storeName"),
                "orgId": item.get("orgId"),
                "extraSession": extra,
                "userPrompt": item.get("userPrompt"),
                "prodReportPreview": item.get("prodReportPreview"),
                "devSessionId": dev_session_id,
                "devTurnId": dev_turn_id,
                "devTaskId": task_id,
                "devStatus": status,
                "devReport": report,
            }
        )
        if status != "succeeded":
            break
    return results


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--manifest",
        type=Path,
        default=ROOT / "cases" / "proj10_manifest_20260614_20260615.json",
    )
    parser.add_argument("--gateway", default=DEFAULT_DEV_GATEWAY)
    parser.add_argument("--dev-proj-id", type=int, default=DEFAULT_DEV_PROJ_ID)
    parser.add_argument(
        "--limit-sessions",
        type=int,
        default=0,
        help="max prod sessions to replay (0 = all)",
    )
    parser.add_argument(
        "--session-id",
        default="",
        help="replay only this prod sessionId",
    )
    parser.add_argument(
        "--out",
        type=Path,
        default=None,
        help="replay result json (default: <manifest-stem>_replay_p27.json)",
    )
    parser.add_argument("--dry-run", action="store_true")
    args = parser.parse_args()

    if not args.manifest.is_file():
        print(f"manifest not found: {args.manifest}", file=sys.stderr)
        return 1

    with args.manifest.open(encoding="utf-8") as f:
        manifest = json.load(f)
    items = manifest.get("items") or []
    groups = group_by_session(items)
    if args.session_id:
        groups = [g for g in groups if g[0] == args.session_id]
    if args.limit_sessions > 0:
        groups = groups[: args.limit_sessions]

    out = args.out or args.manifest.with_name(f"{args.manifest.stem}_replay_p{args.dev_proj_id}.json")
    gateway = args.gateway.rstrip("/")

    print(
        f"replay {len(groups)} sessions ({sum(len(t) for _, t in groups)} turns) "
        f"-> {gateway} projId={args.dev_proj_id}",
        flush=True,
    )

    if args.dry_run:
        for sid, turns in groups[:5]:
            print(f"  {sid}: {len(turns)} turns, bizdate={turns[0].get('bizdate')}")
        if len(groups) > 5:
            print(f"  ... and {len(groups) - 5} more")
        return 0

    all_results: list[dict[str, Any]] = []
    for i, (sid, turns) in enumerate(groups, 1):
        label = f"[{i}/{len(groups)}] {sid} ({len(turns)} turns)"
        print(f"{label} store={turns[0].get('storeName')}", flush=True)
        try:
            session_results = replay_session(gateway, args.dev_proj_id, sid, turns)
            all_results.extend(session_results)
            last = session_results[-1]
            print(
                f"  done status={last.get('devStatus')} "
                f"devSession={last.get('devSessionId')}",
                flush=True,
            )
        except Exception as e:
            print(f"  ERROR: {e}", flush=True)
            all_results.append(
                {
                    "prodSessionId": sid,
                    "devStatus": "error",
                    "error": str(e),
                    "turnCount": len(turns),
                }
            )

    payload = {
        "replayedAtMs": int(time.time() * 1000),
        "manifest": str(args.manifest),
        "devGateway": gateway,
        "devProjId": args.dev_proj_id,
        "sessionCount": len(groups),
        "turnCount": len(all_results),
        "results": all_results,
    }
    out.parent.mkdir(parents=True, exist_ok=True)
    with out.open("w", encoding="utf-8") as f:
        json.dump(payload, f, ensure_ascii=False, indent=2)
    print(f"wrote {out}")
    ok = sum(1 for r in all_results if r.get("devStatus") == "succeeded")
    print(f"succeeded turns: {ok}/{len(all_results)}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

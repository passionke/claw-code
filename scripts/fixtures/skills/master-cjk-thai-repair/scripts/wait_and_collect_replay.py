#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""Wait for observation replay sessions then write replay.json for score_cjk_replay.

Author: kejiqing

Input --sessions-json examples:
  { "sessions": [ {"itemId":"...","sessionId":"..."} ] }
  { "replaySessionIds": [ {"itemId":"...","sessionId":"..."} ] }
  { "replay_session_ids": [ ... ] }  # from repair run field
"""
from __future__ import annotations

import argparse
import json
import time
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path
from typing import Any


def http_get(url: str, timeout: int = 60) -> Any:
    with urllib.request.urlopen(url, timeout=timeout) as resp:
        return json.loads(resp.read().decode("utf-8"))


def normalize_entries(obj: Any) -> list[dict[str, str]]:
    rows: list[Any]
    if isinstance(obj, list):
        rows = obj
    elif isinstance(obj, dict):
        for k in (
            "sessions",
            "results",
            "replaySessionIds",
            "replay_session_ids",
            "items",
        ):
            if isinstance(obj.get(k), list):
                rows = obj[k]
                break
        else:
            rows = []
    else:
        rows = []
    out: list[dict[str, str]] = []
    for r in rows:
        if isinstance(r, str):
            out.append({"itemId": "", "sessionId": r})
            continue
        if not isinstance(r, dict):
            continue
        sid = str(
            r.get("sessionId")
            or r.get("session_id")
            or r.get("replaySessionId")
            or ""
        )
        iid = str(r.get("itemId") or r.get("item_id") or r.get("sourceTurnId") or "")
        if sid:
            out.append({"itemId": iid, "sessionId": sid})
    return out


def latest_turn(base: str, proj: int, session_id: str) -> dict[str, Any] | None:
    url = (
        f"{base.rstrip('/')}/v1/sessions/{urllib.parse.quote(session_id)}"
        f"/turns?projId={proj}"
    )
    try:
        data = http_get(url)
    except urllib.error.HTTPError:
        return None
    turns = data.get("turns") or []
    if not turns:
        return None
    return turns[-1] if isinstance(turns[-1], dict) else None


def terminal(status: str) -> bool:
    return status.lower() in {
        "succeeded",
        "success",
        "completed",
        "failed",
        "error",
        "cancelled",
        "canceled",
    }


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--gateway", required=True)
    ap.add_argument("--observation-proj-id", type=int, required=True)
    ap.add_argument("--sessions-json", required=True)
    ap.add_argument("--out", required=True)
    ap.add_argument("--poll-secs", type=float, default=10.0)
    ap.add_argument("--timeout-secs", type=float, default=1800.0)
    args = ap.parse_args()

    raw = json.loads(Path(args.sessions_json).read_text(encoding="utf-8"))
    entries = normalize_entries(raw)
    if not entries:
        raise SystemExit("no sessions in --sessions-json")

    deadline = time.time() + args.timeout_secs
    results: list[dict[str, Any]] = []
    pending = {e["sessionId"]: e for e in entries}

    while pending and time.time() < deadline:
        done_ids: list[str] = []
        for sid, meta in list(pending.items()):
            turn = latest_turn(args.gateway, args.observation_proj_id, sid)
            if not turn:
                continue
            st = str(turn.get("status") or "")
            if not terminal(st):
                continue
            results.append(
                {
                    "itemId": meta.get("itemId") or "",
                    "sessionId": sid,
                    "turnId": turn.get("turnId"),
                    "status": st,
                    "reportBody": turn.get("reportBody") or "",
                    "userPrompt": turn.get("userPrompt") or "",
                }
            )
            done_ids.append(sid)
        for sid in done_ids:
            pending.pop(sid, None)
        if pending:
            print(
                json.dumps(
                    {"waiting": len(pending), "done": len(results)},
                    ensure_ascii=False,
                ),
                flush=True,
            )
            time.sleep(args.poll_secs)

    out = {
        "observationProjId": args.observation_proj_id,
        "complete": len(pending) == 0,
        "pendingSessionIds": list(pending.keys()),
        "results": results,
    }
    Path(args.out).write_text(json.dumps(out, ensure_ascii=False, indent=2), encoding="utf-8")
    print(
        json.dumps(
            {
                "wrote": args.out,
                "complete": out["complete"],
                "n": len(results),
                "pending": len(pending),
            },
            ensure_ascii=False,
        )
    )
    return 0 if out["complete"] else 2


if __name__ == "__main__":
    raise SystemExit(main())

#!/usr/bin/env python3
# Verify POST /v1/responses stream=true is a true Hub-backed SSE (not await-then-dump).
# Author: kejiqing
#
# Usage:
#   python3 scripts/verify_responses_hub_stream.py
#   GATEWAY=http://127.0.0.1:8088 NGMK_TOKEN=ngmk_… python3 scripts/verify_responses_hub_stream.py
#   python3 scripts/verify_responses_hub_stream.py --proj-id 1 --quick

from __future__ import annotations

import argparse
import json
import os
import sys
import time
import urllib.error
import urllib.request
from typing import Any


def http_json(method: str, url: str, body: dict[str, Any] | None = None, token: str | None = None) -> Any:
    data = None if body is None else json.dumps(body).encode("utf-8")
    headers = {"Accept": "application/json"}
    if body is not None:
        headers["Content-Type"] = "application/json"
    if token:
        headers["Authorization"] = f"Bearer {token}"
    req = urllib.request.Request(url, data=data, headers=headers, method=method)
    with urllib.request.urlopen(req, timeout=60) as resp:
        raw = resp.read().decode("utf-8")
        return json.loads(raw) if raw else None


def mint_ngmk(gateway: str, proj_id: int, admin_token: str | None) -> str:
    url = f"{gateway.rstrip('/')}/v1/projects/{proj_id}/model-api-keys"
    out = http_json(
        "POST",
        url,
        {"name": f"stream-probe-{int(time.time())}", "modelAlias": "agent", "note": "verify_responses_hub_stream"},
        token=admin_token,
    )
    token = (out or {}).get("token")
    if not token or not str(token).startswith("ngmk_"):
        raise RuntimeError(f"mint key failed: {out!r}")
    print(f"minted NGMK_TOKEN={token[:18]}… (save if you need it again)", flush=True)
    return str(token)


def main() -> int:
    ap = argparse.ArgumentParser(description="Probe /v1/responses stream=true timing")
    ap.add_argument("--gateway", default=os.environ.get("GATEWAY", "http://127.0.0.1:8088"))
    ap.add_argument("--proj-id", type=int, default=int(os.environ.get("PROJ_ID", "1")))
    ap.add_argument("--token", default=os.environ.get("NGMK_TOKEN", ""))
    ap.add_argument("--admin-token", default=os.environ.get("CAMT_TOKEN") or os.environ.get("CLAW_ADMIN_TOKEN") or "")
    ap.add_argument("--model", default=os.environ.get("MODEL_ALIAS", "agent"))
    ap.add_argument("--timeout", type=int, default=600)
    ap.add_argument(
        "--quick",
        action="store_true",
        help="short no-tool prompt (still checks response.created arrives early)",
    )
    args = ap.parse_args()

    gateway = args.gateway.rstrip("/")
    token = args.token.strip()
    if not token:
        try:
            token = mint_ngmk(gateway, args.proj_id, args.admin_token.strip() or None)
        except Exception as e:
            print(f"error: need NGMK_TOKEN or mintable /v1/projects/{args.proj_id}/model-api-keys: {e}", file=sys.stderr)
            return 2

    prompt = (
        "只用一句话回答：你是谁。不要调用工具。"
        if args.quick
        else "写一段稍长的说明，并尽量调用工具看一眼工作区文件列表。"
    )
    body = {
        "model": args.model,
        "conversation": f"stream-verify-{int(time.time())}",
        "input": prompt,
        "stream": True,
        "timeout": args.timeout,
    }
    url = f"{gateway}/v1/responses"
    req = urllib.request.Request(
        url,
        data=json.dumps(body).encode("utf-8"),
        headers={
            "Authorization": f"Bearer {token}",
            "Content-Type": "application/json",
            "Accept": "text/event-stream",
        },
        method="POST",
    )

    print(f"POST {url} stream=true proj={args.proj_id} quick={args.quick}", flush=True)
    t0 = time.time()
    created_at: float | None = None
    first_delta_at: float | None = None
    completed_at: float | None = None
    n_delta = 0
    n_tool = 0

    try:
        with urllib.request.urlopen(req, timeout=args.timeout + 30) as resp:
            print(
                f"+{0.0:6.2f}s  HTTP {resp.status}  Content-Type={resp.headers.get('Content-Type')}",
                flush=True,
            )
            sid = resp.headers.get("x-nerogate-session-id")
            if sid:
                print(f"+{0.0:6.2f}s  x-nerogate-session-id={sid}", flush=True)
            buf = b""
            while True:
                chunk = resp.read(1)
                if not chunk:
                    break
                buf += chunk
                while b"\n" in buf:
                    line, buf = buf.split(b"\n", 1)
                    s = line.decode("utf-8", "replace").rstrip("\r")
                    if not (s.startswith("event:") or s.startswith("data:")):
                        continue
                    now = time.time() - t0
                    print(f"+{now:6.2f}s  {s[:200]}", flush=True)
                    if s.startswith("event: response.created"):
                        created_at = now
                    elif s.startswith("event: response.output_text.delta"):
                        n_delta += 1
                        if first_delta_at is None:
                            first_delta_at = now
                    elif s.startswith("event: response.output_item.added"):
                        n_tool += 1
                    elif s.startswith("event: response.completed"):
                        completed_at = now
    except urllib.error.HTTPError as e:
        err = e.read().decode("utf-8", "replace")
        print(f"HTTP {e.code}: {err[:500]}", file=sys.stderr)
        return 1
    except Exception as e:
        print(f"error: {e}", file=sys.stderr)
        return 1

    print("---", flush=True)
    print(
        f"summary: created={created_at} first_delta={first_delta_at} "
        f"completed={completed_at} deltas={n_delta} tools={n_tool}",
        flush=True,
    )
    if created_at is None or completed_at is None:
        print("FAIL: missing response.created or response.completed", file=sys.stderr)
        return 1
    # True stream: created arrives well before completed on a non-trivial turn.
    # Quick mode: still require created before completed (same connection, ordered).
    if completed_at < created_at:
        print("FAIL: completed before created", file=sys.stderr)
        return 1
    gap = completed_at - created_at
    if not args.quick and gap < 0.5 and n_delta <= 1 and n_tool == 0:
        print(
            "WARN: created→completed gap tiny and almost no mid events; "
            "retry without --quick or use a longer tool prompt.",
            file=sys.stderr,
        )
    if created_at <= 2.0:
        print(f"OK: response.created at +{created_at:.2f}s (early SSE open)", flush=True)
    else:
        print(
            f"WARN: response.created late (+{created_at:.2f}s); check proxy buffering",
            file=sys.stderr,
        )
    print(f"OK: created→completed gap={gap:.2f}s deltas={n_delta} tools={n_tool}", flush=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

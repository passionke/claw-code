#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""Fetch Mind FAQ markdown via MCP HTTP and write .mind-export/faq/{id}.md. Author: kejiqing"""

from __future__ import annotations

import json
import sys
import urllib.error
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
MANIFEST = Path(__file__).with_name("mind_faq_manifest.json")
EXPORT_DIR = ROOT / "knowledge/gpos-user-manual/.mind-export/faq"
MCP_CONFIG = Path.home() / ".cursor/mcp.json"
MCP_SERVER = "mind_455785b1-1358-45ce-89c0-5a66e56d7826"


def load_mcp() -> tuple[str, dict[str, str]]:
    cfg = json.loads(MCP_CONFIG.read_text(encoding="utf-8"))
    srv = cfg["mcpServers"][MCP_SERVER]
    url = srv["url"]
    headers = {k: str(v) for k, v in srv.get("headers", {}).items()}
    headers.setdefault("Content-Type", "application/json")
    headers.setdefault("Accept", "application/json, text/event-stream")
    return url, headers


def get_document(url: str, headers: dict[str, str], resource_id: str) -> str:
    payload = {
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {
            "name": "get_document",
            "arguments": {"resourceId": resource_id},
        },
    }
    req = urllib.request.Request(
        url,
        data=json.dumps(payload).encode("utf-8"),
        headers=headers,
        method="POST",
    )
    with urllib.request.urlopen(req, timeout=120) as resp:
        body = json.loads(resp.read().decode("utf-8"))
    if "error" in body:
        raise RuntimeError(body["error"])
    content = body["result"]["content"]
    text = content[0]["text"] if content else ""
    doc = json.loads(text)
    markdown = doc.get("markdown")
    if markdown is None:
        raise RuntimeError("missing markdown field")
    return markdown


def main() -> int:
    if not MANIFEST.exists():
        print(f"missing {MANIFEST}", file=sys.stderr)
        return 2
    url, headers = load_mcp()
    manifest = json.loads(MANIFEST.read_text(encoding="utf-8"))
    EXPORT_DIR.mkdir(parents=True, exist_ok=True)
    written = 0
    failed: list[str] = []
    for doc in manifest.get("docs", []):
        rid = doc["id"]
        out = EXPORT_DIR / f"{rid}.md"
        try:
            markdown = get_document(url, headers, rid)
            out.write_text(markdown, encoding="utf-8")
            written += 1
            print(f"ok {rid}")
        except (urllib.error.URLError, RuntimeError, json.JSONDecodeError, KeyError) as exc:
            failed.append(rid)
            print(f"fail {rid}: {exc}", file=sys.stderr)
    print(f"written={written}")
    print(f"failed={len(failed)}")
    if failed:
        print("failed_ids=" + ",".join(failed))
    return 0 if not failed else 1


if __name__ == "__main__":
    raise SystemExit(main())

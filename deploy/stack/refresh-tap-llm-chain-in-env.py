#!/usr/bin/env python3
"""Sync OPENAI_BASE_URL / INTERNAL_CLAUDE_TAP_HOST from CLAUDE_TAP_* using UPSTREAM_OPENAI_BASE_URL.

Chain (repo-root .env):
  UPSTREAM_OPENAI_BASE_URL  -> real LLM (claude-tap --tap-target in compose)
  OPENAI_BASE_URL           -> http://$CLAUDE_TAP_BIND_HOST:$CLAUDE_TAP_HOST_PORT  (no /v1; claw appends /chat/completions)

Author: kejiqing
"""
from __future__ import annotations

import re
import sys
from pathlib import Path


def has_key(text: str, k: str) -> bool:
    return re.search(rf"^{re.escape(k)}=", text, re.M) is not None


def get_val(text: str, key: str, default: str) -> str:
    m = re.search(rf"^{re.escape(key)}=(.*)$", text, re.M)
    if not m:
        return default
    return m.group(1).strip()


def upsert_line(text: str, key: str, value: str) -> str:
    line = f"{key}={value}"
    if re.search(rf"^{re.escape(key)}=", text, re.M):
        return re.sub(rf"^{re.escape(key)}=.*$", line, text, count=1, flags=re.M)
    return text.rstrip() + "\n" + line + "\n"


def main() -> None:
    if len(sys.argv) != 2:
        print("usage: refresh-tap-llm-chain-in-env.py /path/to/.env", file=sys.stderr)
        sys.exit(2)
    path = Path(sys.argv[1])
    if not path.is_file():
        print(f"error: missing {path}", file=sys.stderr)
        sys.exit(1)
    text = path.read_text(encoding="utf-8")

    bind = get_val(text, "CLAUDE_TAP_BIND_HOST", "host.docker.internal")
    port = get_val(text, "CLAUDE_TAP_HOST_PORT", "8080")

    if not has_key(text, "UPSTREAM_OPENAI_BASE_URL"):
        m = re.search(r"^OPENAI_BASE_URL=(.*)$", text, re.M)
        if m:
            cur = m.group(1).strip()
            if cur and "host.docker.internal" not in cur and "host.containers.internal" not in cur:
                if not cur.startswith("http://claude-tap"):
                    text += f"\nUPSTREAM_OPENAI_BASE_URL={cur}\n"

    if has_key(text, "UPSTREAM_OPENAI_BASE_URL"):
        # Do NOT append /v1: openai_compat::chat_completions_endpoint adds "/chat/completions"
        # to the trimmed base, producing .../chat/completions (tap OK). A base ending in /v1
        # becomes .../v1/chat/completions which this claude-tap stack answers with 404.
        openai_tap = f"http://{bind}:{port}"
        internal_tap = openai_tap
        text = upsert_line(text, "OPENAI_BASE_URL", openai_tap)
        text = upsert_line(text, "INTERNAL_CLAUDE_TAP_HOST", internal_tap)

    path.write_text(text, encoding="utf-8")


if __name__ == "__main__":
    main()

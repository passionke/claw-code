#!/usr/bin/env python3
"""Bootstrap UPSTREAM_OPENAI_BASE_URL in `.claw/claw-llm-runtime.env` when missing.

Human repo `.env` is deploy-only. OPENAI_BASE_URL for workers is in deploy/stack/.claw-worker-llm.env.

Author: kejiqing
"""
from __future__ import annotations

import re
import sys
from pathlib import Path


def has_key(text: str, k: str) -> bool:
    return re.search(rf"^{re.escape(k)}=", text, re.M) is not None


def main() -> None:
    if len(sys.argv) != 2:
        print("usage: refresh-tap-llm-chain-in-env.py /path/to/.env", file=sys.stderr)
        sys.exit(2)
    path = Path(sys.argv[1])
    if not path.is_file():
        print(f"error: missing {path}", file=sys.stderr)
        sys.exit(1)
    text = path.read_text(encoding="utf-8")

    if not has_key(text, "UPSTREAM_OPENAI_BASE_URL"):
        m = re.search(r"^OPENAI_BASE_URL=(.*)$", text, re.M)
        if m:
            cur = m.group(1).strip()
            if cur and "host.docker.internal" not in cur and "host.containers.internal" not in cur:
                if not cur.startswith("http://claude-tap"):
                    text += f"\nUPSTREAM_OPENAI_BASE_URL={cur}\n"
                    path.write_text(text, encoding="utf-8")
                    print(f"note: set UPSTREAM_OPENAI_BASE_URL from legacy OPENAI_BASE_URL in {path}")


if __name__ == "__main__":
    main()

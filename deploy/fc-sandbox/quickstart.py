#!/usr/bin/env python3
# Phase 0 FC connectivity quickstart (Step A). Author: kejiqing
"""Minimal FC sandbox smoke test — no NAS, built-in template."""

from __future__ import annotations

import os
import sys


def main() -> int:
    api_key = os.environ.get("E2B_API_KEY") or os.environ.get("CLAW_FC_API_KEY") or os.environ.get(
        "ALIYUN_E2B_TOKEN"
    )
    if not api_key:
        print("Set E2B_API_KEY, CLAW_FC_API_KEY, or ALIYUN_E2B_TOKEN", file=sys.stderr)
        return 1

    domain = os.environ.get("E2B_DOMAIN", "cn-beijing.e2b.fc.aliyuncs.com")
    template = os.environ.get("CLAW_FC_TEMPLATE", "code-interpreter-v1")

    try:
        from e2b_code_interpreter import Sandbox
    except ImportError:
        print("pip install e2b-code-interpreter", file=sys.stderr)
        return 1

    sandbox = Sandbox.create(template=template, api_key=api_key, domain=domain)
    try:
        result = sandbox.commands.run('python3 -c "print(\'hello from fc\')"')
        print("stdout:", (result.stdout or "").strip())
        print("sandbox_id:", sandbox.sandbox_id)
        if result.exit_code != 0:
            print("stderr:", result.stderr or "", file=sys.stderr)
            return result.exit_code or 1
    finally:
        sandbox.kill()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

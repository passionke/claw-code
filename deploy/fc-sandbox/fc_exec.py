#!/usr/bin/env python3
# FC sandbox envd exec helper for http-gateway-rs (stdin JSON → stdout JSON). Author: kejiqing
"""Run shell scripts inside an existing FC sandbox via e2b-code-interpreter."""

from __future__ import annotations

import json
import sys


def _fail(message: str, code: int = 1) -> None:
    print(json.dumps({"ok": False, "error": message}), flush=True)
    sys.exit(code)


def main() -> None:
    try:
        payload = json.load(sys.stdin)
    except json.JSONDecodeError as exc:
        _fail(f"invalid stdin json: {exc}")

    op = payload.get("op")
    if op != "run_sh":
        _fail(f"unknown op {op!r}")

    api_key = payload.get("api_key") or ""
    domain = payload.get("domain") or "cn-beijing.e2b.fc.aliyuncs.com"
    sandbox_id = payload.get("sandbox_id") or ""
    script = payload.get("script") or ""
    if not api_key.strip():
        _fail("api_key required")
    if not sandbox_id.strip():
        _fail("sandbox_id required")
    if not script.strip():
        _fail("script required")

    try:
        from e2b_code_interpreter import Sandbox
    except ImportError:
        _fail("e2b_code_interpreter not installed; pip install e2b-code-interpreter")

    try:
        sandbox = Sandbox.connect(
            sandbox_id,
            api_key=api_key,
            domain=domain,
        )
        result = sandbox.commands.run(script, timeout=180)
        if result.exit_code != 0:
            stderr = (result.stderr or "").strip()
            stdout = (result.stdout or "").strip()
            detail = stderr or stdout or f"exit {result.exit_code}"
            _fail(f"command exit {result.exit_code}: {detail}")
        print(json.dumps({"ok": True, "stdout": result.stdout or ""}), flush=True)
    except Exception as exc:  # noqa: BLE001 — helper must always emit JSON
        _fail(str(exc))


if __name__ == "__main__":
    main()

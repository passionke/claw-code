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
    if op not in ("run_sh", "exec_solve"):
        _fail(f"unknown op {op!r}")

    api_key = payload.get("api_key") or ""
    domain = payload.get("domain") or "cn-beijing.e2b.fc.aliyuncs.com"
    sandbox_id = payload.get("sandbox_id") or ""
    script = payload.get("script") or ""
    timeout = int(payload.get("timeout") or (600 if op == "exec_solve" else 180))
    if not api_key.strip():
        _fail("api_key required")
    if not sandbox_id.strip():
        _fail("sandbox_id required")
    if op == "run_sh" and not script.strip():
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
        if op == "exec_solve":
            env = payload.get("env") or {}
            exports = "\n".join(
                f'export {k}={json.dumps(str(v))}' for k, v in env.items() if str(v).strip()
            )
            claw_bin = payload.get("claw_bin") or "/usr/local/bin/claw"
            task_file = payload.get("task_file") or "/claw_host_root/gateway-solve-task.json"
            script = (
                "set -eu\n"
                "cd /claw_host_root\n"
                "export HOME=/claw_host_root\n"
                "export XDG_CONFIG_HOME=/claw_host_root/.config\n"
                "export XDG_DATA_HOME=/claw_host_root/.local/share\n"
                f"{exports}\n"
                f"{claw_bin} gateway-solve-once --task-file {task_file}\n"
            )
        result = sandbox.commands.run(script, timeout=timeout)
        if op == "exec_solve":
            print(
                json.dumps(
                    {
                        "ok": True,
                        "exit_code": result.exit_code,
                        "stdout": result.stdout or "",
                        "stderr": result.stderr or "",
                    }
                ),
                flush=True,
            )
            return
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

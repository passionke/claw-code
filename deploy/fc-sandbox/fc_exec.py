#!/usr/bin/env python3
# E2B sandbox envd exec helper (stdin JSON → stdout JSON / NDJSON stream). Author: kejiqing
"""Run shell scripts inside a sandbox via e2b SDK (self-hosted or FC)."""

from __future__ import annotations

import json
import sys


def _fail(message: str, code: int = 1) -> None:
    print(json.dumps({"ok": False, "error": message}), flush=True)
    sys.exit(code)


def _connect_opts(payload: dict) -> dict:
    domain = payload.get("domain") or "10.8.0.9"
    out: dict = {
        "api_key": payload.get("api_key") or "",
        "domain": domain,
    }
    api_url = payload.get("api_url")
    sandbox_url = payload.get("sandbox_url")
    if api_url:
        out["api_url"] = api_url
    if sandbox_url:
        out["sandbox_url"] = sandbox_url
    return out


def _emit_stdout_line(line: str) -> None:
    print(json.dumps({"ev": "stdout_line", "line": line}), flush=True)


class _LineAssembler:
    """Merge envd on_stdout chunks into complete lines (may split mid-line)."""

    def __init__(self) -> None:
        self._buf = ""

    def push(self, chunk: str) -> None:
        if not chunk:
            return
        self._buf += chunk
        while True:
            pos = self._buf.find("\n")
            if pos < 0:
                break
            line = self._buf[: pos + 1]
            self._buf = self._buf[pos + 1 :]
            _emit_stdout_line(line)

    def flush_tail(self) -> None:
        if self._buf:
            _emit_stdout_line(self._buf)
            self._buf = ""


def _run_streaming(sandbox, script: str, timeout: int):
    assembler = _LineAssembler()
    stdout_parts: list[str] = []
    stderr_parts: list[str] = []

    def on_stdout(data) -> None:
        text = data if isinstance(data, str) else str(data)
        stdout_parts.append(text)
        assembler.push(text)

    def on_stderr(data) -> None:
        text = data if isinstance(data, str) else str(data)
        stderr_parts.append(text)

    result = sandbox.commands.run(
        script,
        timeout=timeout,
        on_stdout=on_stdout,
        on_stderr=on_stderr,
    )
    assembler.flush_tail()
    stderr = result.stderr or "".join(stderr_parts)
    stdout = result.stdout if result.stdout else "".join(stdout_parts)
    return result, stdout, stderr


def main() -> None:
    try:
        payload = json.load(sys.stdin)
    except json.JSONDecodeError as exc:
        _fail(f"invalid stdin json: {exc}")

    op = payload.get("op")
    if op not in ("run_sh", "exec_solve"):
        _fail(f"unknown op {op!r}")

    sandbox_id = payload.get("sandbox_id") or ""
    script = payload.get("script") or ""
    timeout = int(payload.get("timeout") or (600 if op == "exec_solve" else 180))
    if not (payload.get("api_key") or "").strip():
        _fail("api_key required")
    if not sandbox_id.strip():
        _fail("sandbox_id required")
    if op == "run_sh" and not script.strip():
        _fail("script required")

    try:
        from e2b_code_interpreter import Sandbox
    except ImportError:
        _fail("e2b_code_interpreter not installed; pip install e2b-code-interpreter")

    connect = _connect_opts(payload)
    try:
        sandbox = Sandbox.connect(sandbox_id, **connect)
        if op == "exec_solve":
            env = payload.get("env") or {}
            exports = "\n".join(
                f'export {k}={json.dumps(str(v))}' for k, v in env.items() if str(v).strip()
            )
            claw_bin = payload.get("claw_bin") or "claw"
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
            result, stdout, stderr = _run_streaming(sandbox, script, timeout)
            print(
                json.dumps(
                    {
                        "ok": True,
                        "exit_code": result.exit_code,
                        "stdout": stdout,
                        "stderr": stderr,
                    }
                ),
                flush=True,
            )
            return
        result, stdout, stderr = _run_streaming(sandbox, script, timeout)
        if result.exit_code != 0:
            stderr = (stderr or "").strip()
            stdout = (stdout or "").strip()
            detail = stderr or stdout or f"exit {result.exit_code}"
            _fail(f"command exit {result.exit_code}: {detail}")
        print(
            json.dumps(
                {
                    "ok": True,
                    "exit_code": result.exit_code,
                    "stdout": stdout,
                    "stderr": stderr,
                }
            ),
            flush=True,
        )
    except Exception as exc:  # noqa: BLE001 — helper must always emit JSON
        _fail(str(exc))


if __name__ == "__main__":
    main()

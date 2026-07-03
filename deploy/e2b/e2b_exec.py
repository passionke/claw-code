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
    domain = payload.get("domain") or "supone.top"
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


def _run_as_claw_user_script(inner: str) -> str:
    """e2b envd runs `commands.run` as `user` (uid 1001); NAS session trees are claw (1000).

    Without this wrapper, inline writes to `/claw_sessions/{segment}/gateway-solve-task.json`
    fail with bash redirect Permission denied. Author: kejiqing
    """
    return (
        "set -eu\n"
        "sudo -u claw bash <<'CLAW_SOLVE_EOF'\n"
        f"{inner}"
        "CLAW_SOLVE_EOF\n"
    )


def _inline_writes_sh(task_file: str, task_json, session_jsonl, session_root: str) -> str:
    """Shell snippet that lands per-turn inputs onto the session mount.

    Content is base64-encoded (shell-safe charset) and decoded in-guest. Author: kejiqing
    """
    import base64

    lines: list[str] = []
    root = session_root.rstrip("/") or "/claw_host_root"
    if task_json is not None and str(task_json) != "":
        b = base64.b64encode(str(task_json).encode("utf-8")).decode("ascii")
        lines.append(f"mkdir -p {root}")
        lines.append(f"printf %s '{b}' | base64 -d > {task_file}")
    if session_jsonl is not None and str(session_jsonl) != "":
        b = base64.b64encode(str(session_jsonl).encode("utf-8")).decode("ascii")
        lines.append(f"mkdir -p {root}/.claw")
        lines.append(
            f"printf %s '{b}' | base64 -d > {root}/.claw/gateway-solve-session.jsonl"
        )
    return ("\n".join(lines) + "\n") if lines else ""


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
    # connect() without an explicit timeout makes the e2b SDK reset the sandbox
    # lifetime to its 300s default; keep the create-time lifetime instead.
    sandbox_timeout = int(payload.get("sandbox_timeout") or 0)
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
        if sandbox_timeout > 0:
            sandbox = Sandbox.connect(sandbox_id, timeout=sandbox_timeout, **connect)
        else:
            sandbox = Sandbox.connect(sandbox_id, **connect)
        if op == "exec_solve":
            env = payload.get("env") or {}
            exports = "\n".join(
                f'export {k}={json.dumps(str(v))}' for k, v in env.items() if str(v).strip()
            )
            claw_bin = payload.get("claw_bin") or "claw"
            session_segment = str(payload.get("session_segment") or "").strip()
            session_root = str(payload.get("session_root") or "").strip()
            if not session_root and session_segment:
                session_root = f"/claw_sessions/{session_segment}"
            if not session_root:
                session_root = "/claw_host_root"
            task_file = payload.get("task_file") or f"{session_root}/gateway-solve-task.json"
            inline = _inline_writes_sh(
                task_file,
                payload.get("task_json"),
                payload.get("session_jsonl"),
                session_root,
            )
            inner = (
                "set -eu\n"
                f"cd {session_root}\n"
                f"export HOME={session_root}\n"
                f"export XDG_CONFIG_HOME={session_root}/.config\n"
                f"export XDG_DATA_HOME={session_root}/.local/share\n"
                "export CLAW_PROJECT_CONFIG_ROOT=/claw_ds/project_home_def\n"
                f"{inline}"
                f"{exports}\n"
                f"{claw_bin} gateway-solve-once --task-file {task_file}\n"
            )
            script = _run_as_claw_user_script(inner)
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

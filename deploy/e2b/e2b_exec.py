#!/usr/bin/env python3
# E2B sandbox envd exec helper (stdin JSON → stdout JSON / NDJSON stream). Author: kejiqing
"""Run shell scripts inside a sandbox via e2b SDK (self-hosted or FC)."""

from __future__ import annotations

import json
import sys
from pathlib import PurePosixPath
from tempfile import SpooledTemporaryFile

SPOOLED_FALLBACK_MAX_BYTES = 2 * 1024 * 1024


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
    """Run solve as the worker exec user (e2b envd `user`, uid 1000).

    Legacy worker images expose a `claw` account; self-hosted e2b templates use
    `user` (uid 1000) with NAS trees owned by `user`. Prefer `claw` when present.
    Author: kejiqing
    """
    return (
        "set -eu\n"
        "if id claw >/dev/null 2>&1; then\n"
        "  sudo -u claw bash <<'CLAW_SOLVE_EOF'\n"
        f"{inner}"
        "CLAW_SOLVE_EOF\n"
        "else\n"
        f"{inner}"
        "fi\n"
    )


def _env_exports_sh(env: dict) -> str:
    """Inline shell exports for worker LLM env (shared by exec_solve and run_sh). Author: kejiqing"""
    if not env:
        return ""
    lines = [
        f"export {k}={json.dumps(str(v))}"
        for k, v in env.items()
        if str(v).strip()
    ]
    return ("\n".join(lines) + "\n") if lines else ""


def _prepend_env_exports(script: str, env: dict) -> str:
    exports = _env_exports_sh(env)
    if not exports:
        return script
    return f"set -eu\n{exports}{script}"


def _normalize_sse_log_paths_for_session(env: dict, session_root: str) -> dict:
    """Force SSE trace/debug logs into current session root. Author: kejiqing"""
    if not env or not session_root.strip():
        return env
    out = dict(env)
    for key in ("CLAW_SSE_BURST_LOG_FILE", "CLAW_SSE_LOG_FILE"):
        raw = str(out.get(key) or "").strip()
        if not raw:
            continue
        p = PurePosixPath(raw)
        if p.is_absolute():
            # Guardrail: `/claw_sessions/<file>` is sessions root, not current session root.
            if (
                len(p.parts) == 3
                and p.parts[0] == "/"
                and p.parts[1] == "claw_sessions"
            ):
                out[key] = str(PurePosixPath(session_root) / p.name)
            continue
        out[key] = str(PurePosixPath(session_root) / p)
    return out


def _emit_stdout_line(line: str) -> None:
    print(json.dumps({"ev": "stdout_line", "line": line}), flush=True)


class _SpooledTextFallback:
    """Bounded-memory text fallback; spills to temp file past threshold. Author: kejiqing"""

    def __init__(self) -> None:
        self._file = SpooledTemporaryFile(
            max_size=SPOOLED_FALLBACK_MAX_BYTES,
            mode="w+",
            encoding="utf-8",
        )

    def write(self, text: str) -> None:
        if text:
            self._file.write(text)

    def read_all(self) -> str:
        self._file.seek(0)
        return self._file.read()

    def close(self) -> None:
        self._file.close()


class _LineAssembler:
    """Merge envd on_stdout chunks into complete lines (may split mid-line)."""

    def __init__(self) -> None:
        self._buf = ""

    def push(self, chunk: str) -> int:
        if not chunk:
            return 0
        combined = self._buf + chunk
        parts = combined.splitlines(keepends=True)
        emitted = 0
        self._buf = ""
        for part in parts:
            if part.endswith("\n"):
                _emit_stdout_line(part)
                emitted += 1
            else:
                self._buf = part
        return emitted

    def flush_tail(self) -> None:
        if self._buf:
            _emit_stdout_line(self._buf)
            self._buf = ""


def _run_streaming(sandbox, script: str, timeout: int):
    assembler = _LineAssembler()
    stdout_fallback = _SpooledTextFallback()
    stderr_fallback = _SpooledTextFallback()
    stdout_obs = {"chunks": 0, "lines": 0, "bytes": 0}

    def on_stdout(data) -> None:
        text = data if isinstance(data, str) else str(data)
        stdout_fallback.write(text)
        stdout_obs["chunks"] += 1
        stdout_obs["bytes"] += len(text.encode("utf-8"))
        stdout_obs["lines"] += assembler.push(text)

    def on_stderr(data) -> None:
        text = data if isinstance(data, str) else str(data)
        stderr_fallback.write(text)

    try:
        result = sandbox.commands.run(
            script,
            timeout=timeout,
            on_stdout=on_stdout,
            on_stderr=on_stderr,
        )
        assembler.flush_tail()
        stderr = result.stderr or stderr_fallback.read_all()
        stdout = result.stdout if result.stdout else stdout_fallback.read_all()
        return result, stdout, stderr, stdout_obs
    finally:
        stdout_fallback.close()
        stderr_fallback.close()


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
    # exec_solve: gateway should pass timeout; keep 600 as last-resort default when
    # older callers omit it. Not controlled by env. Author: kejiqing
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
            claw_bin = payload.get("claw_bin") or "claw"
            session_segment = str(payload.get("session_segment") or "").strip()
            session_root = str(payload.get("session_root") or "").strip()
            if not session_root and session_segment:
                session_root = f"/claw_sessions/{session_segment}"
            if not session_root:
                session_root = "/claw_host_root"
            env = _normalize_sse_log_paths_for_session(env, session_root)
            exports = _env_exports_sh(env)
            task_file = payload.get("task_file") or f"{session_root}/gateway-solve-task.json"
            # Task body is on NAS (gateway nas-api PUT); never embed in shell (ARG_MAX). Author: kejiqing
            inner = (
                "set -eu\n"
                f"cd {session_root}\n"
                f"export HOME={session_root}\n"
                f"export CLAW_GATEWAY_WORK_ROOT={session_root}\n"
                f"export XDG_CONFIG_HOME={session_root}/.config\n"
                f"export XDG_DATA_HOME={session_root}/.local/share\n"
                "export CLAW_PROJECT_CONFIG_ROOT=/claw_ds/project_home_def\n"
                f'test -f {task_file} || {{ echo "missing task file: {task_file}" >&2; exit 1; }}\n'
                f"{exports}\n"
                f"{claw_bin} gateway-solve-once --task-file {task_file}\n"
            )
            script = _run_as_claw_user_script(inner)
            result, stdout, stderr, stdout_obs = _run_streaming(sandbox, script, timeout)
            print(
                json.dumps(
                    {
                        "ok": True,
                        "exit_code": result.exit_code,
                        "stdout": stdout,
                        "stderr": stderr,
                        "stdoutObs": stdout_obs,
                    }
                ),
                flush=True,
            )
            return
        run_env = payload.get("env") or {}
        script = _prepend_env_exports(script, run_env)
        result, stdout, stderr, _stdout_obs = _run_streaming(sandbox, script, timeout)
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

#!/usr/bin/env python3
"""CI cluster gate: dual gateway solve + cross-gateway session. Author: kejiqing"""
from __future__ import annotations

import json
import os
import re
import shutil
import subprocess
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path
from typing import Any

LIB_DIR = Path(__file__).resolve().parent
PODMAN_DIR = LIB_DIR.parent
REPO_ROOT = PODMAN_DIR.parent.parent
ENV_A = REPO_ROOT / ".env"
ENV_B = REPO_ROOT / ".env.ci-node-b"
SESSION_ID_RE = re.compile(r"^[0-9a-f]{32}$")


def fail(msg: str) -> None:
    print(f"CI CLUSTER SOLVE E2E FAIL: {msg}", file=sys.stderr)
    sys.exit(1)


def ok(msg: str) -> None:
    print(f"CI CLUSTER SOLVE E2E OK: {msg}")


def load_dotenv(path: Path) -> dict[str, str]:
    if not path.is_file():
        fail(f"missing {path}")
    out: dict[str, str] = {}
    for line in path.read_text(encoding="utf-8").splitlines():
        line = line.strip()
        if not line or line.startswith("#") or "=" not in line:
            continue
        k, _, v = line.partition("=")
        out[k.strip()] = v.strip().strip("'\"")
    return out


def http_json(method: str, url: str, body: dict[str, Any] | None = None, timeout: float = 120) -> Any:
    data = None
    headers: dict[str, str] = {}
    if body is not None:
        data = json.dumps(body, ensure_ascii=False).encode("utf-8")
        headers["Content-Type"] = "application/json"
    req = urllib.request.Request(url, data=data, method=method, headers=headers)
    try:
        with urllib.request.urlopen(req, timeout=timeout) as resp:
            raw = resp.read().decode("utf-8")
            return json.loads(raw) if raw else {}
    except urllib.error.HTTPError as e:
        err = e.read().decode("utf-8", errors="replace")[:800]
        fail(f"{method} {url} HTTP {e.code}: {err}")


def wait_readyz(port: int, attempts: int = 45) -> None:
    for i in range(1, attempts + 1):
        try:
            with urllib.request.urlopen(f"http://127.0.0.1:{port}/readyz", timeout=2) as _:
                print(f"gateway clawTap ready (/readyz attempt {i}/{attempts})", file=sys.stderr)
                return
        except urllib.error.HTTPError as e:
            reason = e.read().decode("utf-8", errors="replace")[:200]
            print(f"waiting gateway /readyz ({i}/{attempts}): {reason}…", file=sys.stderr)
        except OSError as e:
            print(f"waiting gateway /readyz ({i}/{attempts}): {e}…", file=sys.stderr)
        time.sleep(2)
    fail(f"gateway /readyz not ready on :{port} after {attempts} attempts")


def ensure_project(port: int, proj_id: int) -> None:
    code = 0
    try:
        with urllib.request.urlopen(
            f"http://127.0.0.1:{port}/v1/project/config/{proj_id}", timeout=15
        ):
            code = 200
    except urllib.error.HTTPError as e:
        code = e.code
    if code != 200:
        http_json("POST", f"http://127.0.0.1:{port}/v1/projects", {"projId": proj_id})
    http_json("POST", f"http://127.0.0.1:{port}/v1/init", {"projId": proj_id})


def set_worker_profile(port: int, proj_id: int, mode: str) -> None:
    if mode not in ("strict", "relaxed"):
        fail(f"worker isolation mode must be strict or relaxed (got {mode})")
    print(f"==> e2e set proj={proj_id} workerProfileJson.mode={mode} (gateway :{port})", file=sys.stderr)
    cfg = http_json("GET", f"http://127.0.0.1:{port}/v1/project/config/{proj_id}")
    body = {
        "contentRev": cfg.get("contentRev") or "",
        "rulesJson": cfg.get("rulesJson") or [],
        "mcpServersJson": cfg.get("mcpServersJson") or {},
        "skillsSourcesJson": cfg.get("skillsSourcesJson") or [],
        "skillsJson": cfg.get("skillsJson") or [],
        "allowedToolsJson": cfg.get("allowedToolsJson") or [],
        "claudeMd": cfg.get("claudeMd"),
        "gitSyncJson": cfg.get("gitSyncJson") or {},
        "solvePreflightJson": cfg.get("solvePreflightJson") or {},
        "solveOrchestrationJson": cfg.get("solveOrchestrationJson") or {},
        "extraSessionFieldsJson": cfg.get("extraSessionFieldsJson") or [],
        "promptLimitsJson": cfg.get("promptLimitsJson") or {},
        "workerProfileJson": {"mode": mode},
    }
    http_json("PUT", f"http://127.0.0.1:{port}/v1/project/config/{proj_id}", body)
    got = http_json("GET", f"http://127.0.0.1:{port}/v1/project/config/{proj_id}")
    got_mode = ((got.get("workerProfileJson") or {}).get("mode") or "").strip()
    if got_mode != mode:
        fail(f"workerProfileJson.mode={got_mode!r} expected {mode!r}")


def build_solve_body(port: int, proj_id: int, prompt: str, session_id: str | None) -> dict[str, Any]:
    cfg = http_json("GET", f"http://127.0.0.1:{port}/v1/project/config/{proj_id}")
    extra = {
        "tenant_code": "GPOS",
        "solution_code": "restaurant",
        "biz_type": "BOSS_REPORT",
        "client_origin": "gateway-admin",
    }
    for f in cfg.get("extraSessionFieldsJson") or []:
        if isinstance(f, str) and f.strip():
            extra[f.strip()] = ""
    body: dict[str, Any] = {"projId": proj_id, "userPrompt": prompt, "extraSession": extra}
    if session_id:
        body["sessionId"] = session_id
    return body


def assert_task(
    task: dict[str, Any],
    label: str,
    *,
    expect_pool_id: str | None = None,
    expect_isolation: str | None = None,
) -> None:
    if expect_pool_id is not None:
        got = (task.get("poolId") or "").strip()
        if got != expect_pool_id:
            fail(f"{label} poolId={got!r} expected {expect_pool_id!r}")
    if expect_isolation is not None:
        got = (task.get("workerProfile") or "").strip()
        if got != expect_isolation:
            fail(f"{label} workerProfile={got!r} expected {expect_isolation!r}")


def solve_e2e(
    port: int,
    proj_id: int,
    prompt: str = "ping",
    *,
    pool_id: str | None = None,
    worker_profile: str | None = None,
    expect_isolation: str | None = None,
    session_id: str | None = None,
) -> str:
    """POST solve_async, poll to terminal; return sessionId from solve_async response."""
    if worker_profile:
        set_worker_profile(port, proj_id, worker_profile)
    wait_readyz(port)
    ensure_project(port, proj_id)

    body = build_solve_body(port, proj_id, prompt, session_id)
    print(f"POST /v1/solve_async gateway :{port}", file=sys.stderr)
    print(json.dumps(body, ensure_ascii=False), file=sys.stderr)
    task = http_json("POST", f"http://127.0.0.1:{port}/v1/solve_async", body)
    print(json.dumps(task, ensure_ascii=False), file=sys.stderr)
    assert_task(task, "solve_async", expect_pool_id=pool_id, expect_isolation=expect_isolation)

    sid = str(task.get("sessionId") or "").strip()
    if not SESSION_ID_RE.fullmatch(sid):
        fail(f"solve_async returned invalid sessionId: {sid!r}")

    task_id = str(task["taskId"])
    for _ in range(120):
        time.sleep(2)
        polled = http_json("GET", f"http://127.0.0.1:{port}/v1/tasks/{task_id}")
        status = polled.get("status")
        print(f"poll status={status}", file=sys.stderr)
        if status in ("succeeded", "failed"):
            print(json.dumps(polled, ensure_ascii=False, indent=2), file=sys.stderr)
            if status != "succeeded":
                fail(f"solve task {task_id} failed on gateway :{port}")
            assert_task(polled, "task poll", expect_pool_id=pool_id, expect_isolation=expect_isolation)
            return sid
    fail(f"timeout waiting task {task_id} on gateway :{port}")


def container_runtime() -> str:
    for cmd in ("podman", "docker"):
        if shutil.which(cmd):
            return cmd
    fail("need docker or podman")


def workspace_writable(ws: Path, label: str, rt: str, uid: int, gid: int) -> None:
    img = os.environ.get("CLAW_CHOWN_RUNNER_IMAGE", "docker.1ms.run/library/alpine:3.20")
    if not ws.is_dir():
        fail(f"{label} workspace missing: {ws}")
    subprocess.run(
        [
            rt,
            "run",
            "--rm",
            "-u",
            f"{uid}:{gid}",
            "-v",
            f"{ws}:/w:rw",
            img,
            "sh",
            "-c",
            "touch /w/.ci-cluster-write-probe && rm -f /w/.ci-cluster-write-probe",
        ],
        check=True,
    )


def main() -> None:
    env_a = load_dotenv(ENV_A)
    env_b = load_dotenv(ENV_B)

    gw_a = int(env_a.get("GATEWAY_HOST_PORT", "18088"))
    gw_b = int(env_b.get("GATEWAY_HOST_PORT", "18089"))
    pool_a = env_a.get("CLAW_POOL_ID", "pool-sunmi-ci-01")
    pool_b = env_b.get("CLAW_POOL_ID", "pool-sunmi-ci-02")
    pool_http_b = int(env_b.get("CLAW_POOL_HTTP_PORT", "9964"))
    proj_id = int(
        env_a.get("CLAW_BOOTSTRAP_PROJ_ID") or env_a.get("CLAW_BOOTSTRAP_DS_ID") or "1"
    )
    ws_a = PODMAN_DIR / "claw-workspace"
    ws_b = PODMAN_DIR / "claw-workspace-ci-b"
    uid = int(env_a.get("CLAW_WORKER_UID", "1000"))
    gid = int(env_a.get("CLAW_WORKER_GID", "1000"))
    rt = container_runtime()

    print(f"==> [1/7] per-gateway workspace (A={ws_a} B={ws_b})")
    if ws_a == ws_b:
        fail(f"node A/B must use separate workspace binds (got same {ws_a})")

    print(f"==> [2/7] each workspace writable as gateway uid {uid}")
    workspace_writable(ws_a, "node A", rt, uid, gid)
    workspace_writable(ws_b, "node B", rt, uid, gid)

    print(f"==> [3/7] node B pool HTTP :{pool_http_b}")
    try:
        with urllib.request.urlopen(
            f"http://127.0.0.1:{pool_http_b}/healthz/live-report", timeout=5
        ):
            pass
    except OSError as e:
        fail(f"node B pool not reachable on :{pool_http_b}: {e}")

    print(f"==> [4/7] node B gateway solve strict ×2 (:{gw_b} pool={pool_b})")
    solve_e2e(gw_b, proj_id, pool_id=pool_b, expect_isolation="strict")
    solve_e2e(gw_b, proj_id, pool_id=pool_b, expect_isolation="strict")

    print("==> [5/7] cross-gateway session: created on A, first turn on B (local dir recreate)")
    session_id = solve_e2e(gw_a, proj_id, pool_id=pool_a, expect_isolation="strict")
    print(f"    sessionId={session_id}", file=sys.stderr)
    solve_e2e(
        gw_b,
        proj_id,
        pool_id=pool_b,
        expect_isolation="strict",
        session_id=session_id,
    )

    print(f"==> [6/7] node B gateway solve relaxed (:{gw_b} pool={pool_b})")
    solve_e2e(
        gw_b,
        proj_id,
        pool_id=pool_b,
        worker_profile="relaxed",
        expect_isolation="relaxed",
    )

    print(f"==> [7/7] node A still solves after cluster (:{gw_a} pool={pool_a})")
    set_worker_profile(gw_a, proj_id, "strict")
    solve_e2e(gw_a, proj_id, pool_id=pool_a, expect_isolation="strict")

    ok("per-gateway workspace + node B solve + cross-gateway session passed")


if __name__ == "__main__":
    main()

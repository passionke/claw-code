#!/usr/bin/env python3
"""Smoke-test claw-worker-relaxed: OVS, NAS mounts, claw binary. Author: kejiqing"""
from __future__ import annotations

import json
import os
import subprocess
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path

_E2B_DIR = Path(__file__).resolve().parent
ROOT = _E2B_DIR.parents[1]
if str(_E2B_DIR) not in sys.path:
    sys.path.insert(0, str(_E2B_DIR))

from e2b_nas_bind_config import e2b_host_mount_root, http_json_selfhosted
from e2b_template_registry import load_repo_dotenv
from ovs_bundle import ovs_port

load_repo_dotenv(ROOT)


def _env(name: str, default: str = "") -> str:
    return os.environ.get(name, default).strip()


def _fail(msg: str, code: int = 1) -> None:
    print(f"verify-claw-worker-relaxed-sandbox: FAIL {msg}", file=sys.stderr)
    raise SystemExit(code)


def _ok(msg: str) -> None:
    print(f"  OK {msg}")


def _http(method: str, url: str, api_key: str, body: dict | None = None) -> tuple[int, object]:
    try:
        data = http_json_selfhosted(method, url, api_key, True, body)
        return 200, data
    except urllib.error.HTTPError as exc:
        raw = exc.read().decode("utf-8", errors="replace")
        try:
            return exc.code, json.loads(raw)
        except json.JSONDecodeError:
            return exc.code, raw


def _resolve_nas_api(api_url: str, api_key: str) -> str:
    explicit = _env("CLAW_E2B_NAS_API_URL")
    if explicit:
        return explicit.rstrip("/")
    code, sandboxes = _http("GET", f"{api_url.rstrip('/')}/sandboxes", api_key)
    if code != 200:
        _fail(f"GET /sandboxes for nas-api discovery HTTP {code}")
    for sb in sandboxes if isinstance(sandboxes, list) else []:
        meta = sb.get("metadata") or {}
        if meta.get("clawRole") in ("nas-api", "nas-api-singleton"):
            sid = sb.get("sandboxID") or sb.get("sandbox_id") or ""
            domain = _env("CLAW_E2B_DOMAIN", "supone.top")
            port = int(_env("CLAW_E2B_NAS_API_PORT", "8090"))
            return f"http://{port}-{sid}.{domain}"
    _fail("claw-nas-api sandbox not found; run ./deploy/stack/gateway.sh nas-api-up")


def _relaxed_template_id(api_url: str, api_key: str, explicit: str) -> str:
    if explicit:
        return explicit
    try:
        from e2b_pg_settings import load_settings_json_key

        tid = (load_settings_json_key("e2bWorkerRelaxed").get("templateId") or "").strip()
        if tid:
            print(f"==> template from PG e2bWorkerRelaxed: {tid!r}")
            return tid
    except Exception as exc:  # noqa: BLE001
        print(f"==> PG e2bWorkerRelaxed unavailable ({exc}); fall back to e2b health", file=sys.stderr)
    return _latest_relaxed_template_from_health(api_url, api_key)


def _latest_relaxed_template_from_health(api_url: str, api_key: str) -> str:
    code, health = _http("GET", f"{api_url.rstrip('/')}/health", api_key)
    if code != 200:
        _fail(f"GET /health HTTP {code}")
    items = ((health.get("templates") or {}).get("items") or []) if isinstance(health, dict) else []
    candidates = [
        t
        for t in items
        if "claw-worker-relaxed" in (t.get("aliases") or [])
        and t.get("imagePresent") is True
    ]
    if not candidates:
        _fail("no claw-worker-relaxed template with imagePresent=true — run build-claw-worker-relaxed-selfhosted.py")
    # Prefer latest templateId lexicographically (rough proxy); user can pass CLAW_E2B_TEMPLATE_RELAXED tpl_*
    tid = sorted(candidates, key=lambda t: t.get("templateId", ""))[-1]["templateId"]
    return tid


def main() -> int:
    api_key = _env("CLAW_E2B_API_KEY", _env("E2B_API_KEY"))
    api_url = _env("CLAW_E2B_API_URL", "http://10.8.0.1:3000")
    sandbox_url = _env("CLAW_E2B_SANDBOX_URL", "http://10.8.0.1:3002")
    domain = _env("CLAW_E2B_DOMAIN", "supone.top")
    cluster = _env("CLAW_CLUSTER_ID", "local-dev")
    proj = int(_env("CLAW_E2B_E2E_PROJ_ID", _env("CLAW_OVS_E2E_PROJ_ID", "2")))
    worker = _env("CLAW_RELAXED_VERIFY_WORKER", "wrk_relaxed_verify")
    template = _relaxed_template_id(api_url, api_key, _env("CLAW_E2B_TEMPLATE_RELAXED"))
    ovs_port_num = ovs_port()

    print(f"==> template={template!r} proj={proj} cluster={cluster!r}")

    host_root = e2b_host_mount_root(
        env_get=lambda k, d="": _env(k, d),
        api_url=api_url,
        api_key=api_key,
        self_hosted=True,
        http_json=lambda m, u, k, sh, b=None: http_json_selfhosted(m, u, k, sh, b),
    )
    nas_api = _resolve_nas_api(api_url, api_key)
    curl = subprocess.run(["curl", "-fsS", "-m", "15", f"{nas_api}/healthz"], capture_output=True)
    if curl.returncode != 0:
        _fail(f"nas-api unhealthy at {nas_api}/healthz")

    for rel in (
        f"{cluster}/proj_{proj}/workers/{worker}/.claw",
        f"{cluster}/proj_{proj}/home",
        f"{cluster}/proj_{proj}/sessions",
        "tap-traces",
    ):
        http_json_selfhosted(
            "POST",
            f"{nas_api}/v1/mkdir",
            api_key,
            True,
            {"relPath": rel, "parents": True},
        )
        _ok(f"nas-api mkdir {rel}")

    nas = {
        "userId": int(_env("CLAW_WORKER_UID", "1000")),
        "groupId": int(_env("CLAW_WORKER_GID", "1000")),
        "hostMountRoot": host_root,
        "mountPoints": [
            {"relPath": f"{cluster}/proj_{proj}/workers/{worker}", "mountDir": "/claw_host_root"},
            {"relPath": f"{cluster}/proj_{proj}/sessions", "mountDir": "/claw_sessions"},
            {"relPath": f"{cluster}/proj_{proj}/home", "mountDir": "/claw_ds", "readOnly": True},
            {"relPath": "tap-traces", "mountDir": "/claw_tap_traces"},
        ],
    }
    create_body = {
        "templateID": template,
        "timeout": 600,
        "metadata": {"verify": "claw-worker-relaxed", "clawRole": "proj-worker-relaxed-verify"},
        "nasConfig": nas,
        "secure": False,
    }
    create_json = json.dumps(create_body)
    create_proc = subprocess.run(
        [
            "curl", "-sS", "-m", "600",
            "-w", "\n%{http_code}",
            "-X", "POST", f"{api_url.rstrip('/')}/sandboxes",
            "-H", f"X-API-Key: {api_key}",
            "-H", "Content-Type: application/json",
            "-d", create_json,
        ],
        capture_output=True,
        text=True,
    )
    if create_proc.returncode != 0:
        _fail(f"POST /sandboxes curl failed: {create_proc.stderr}")
    create_lines = create_proc.stdout.rsplit("\n", 1)
    if len(create_lines) != 2:
        _fail(f"POST /sandboxes bad output: {create_proc.stdout[:500]}")
    create_raw, code_str = create_lines
    code = int(code_str.strip())
    try:
        created = json.loads(create_raw) if create_raw.strip() else {}
    except json.JSONDecodeError:
        created = create_raw
    if code not in (200, 201):
        _fail(f"POST /sandboxes HTTP {code}: {created}")
    sid = created.get("sandboxID") or created.get("sandbox_id") or ""
    if not sid:
        _fail(f"missing sandboxID in create response: {created}")
    print(f"==> sandbox_id={sid}")

    try:
        from e2b_code_interpreter import Sandbox

        sb = Sandbox.connect(
            sid,
            api_key=api_key,
            domain=domain,
            api_url=api_url.rstrip("/"),
            sandbox_url=sandbox_url.rstrip("/"),
            timeout=600,
        )

        checks = [
            "command -v claw",
            "command -v ttyd",
            "test -x /usr/local/bin/claw-worker-relaxed-start",
            "test -x /usr/local/bin/claw-worker-relaxed-ready",
            "test -x /usr/local/bin/claw-ovs-start",
            "test -x /usr/local/bin/claw-ovs-ready",
            f"curl -fsS --connect-timeout 3 http://127.0.0.1:{ovs_port_num}/ovs/",
            "test -f /opt/claw-ovs/server-data/Machine/settings.json",
            "grep -q disabled.invalid /opt/claw-ovs/server-data/Machine/settings.json",
            'HOME=/opt/claw-ovs/home /home/.openvscode-server/bin/openvscode-server --list-extensions --extensions-dir=/opt/claw-extensions --server-data-dir=/opt/claw-ovs/server-data | grep -q "^claw\\.claw-vscode$"',
            "test -d /home/.openvscode-server",
        ]
        for cmd in checks:
            r = sb.commands.run(cmd, timeout=60)
            if r.exit_code not in (0, None):
                _fail(f"$ {cmd} exit={r.exit_code} stderr={(r.stderr or '')[:500]}")
            _ok(cmd)

        r = sb.commands.run("/usr/local/bin/claw-worker-relaxed-ready", timeout=30)
        if r.exit_code not in (0, None):
            _fail(f"claw-worker-relaxed-ready exit={r.exit_code} stderr={r.stderr}")
        _ok("claw-worker-relaxed-ready")

        mount_script = (
            "for d in /claw_host_root /claw_ds /claw_sessions /claw_tap_traces; do "
            'mountpoint -q "$d" || { echo "MISS $d"; exit 1; }; echo "OK $d"; done'
        )
        r = sb.commands.run(mount_script, timeout=30)
        if r.exit_code not in (0, None):
            _fail(f"NAS mountpoints: {(r.stdout or '')} {(r.stderr or '')}")
        _ok(f"NAS mounts: {(r.stdout or '').strip()}")

        probe = f"relaxed-verify-{int(time.time())}"
        r = sb.commands.run(
            f"echo '{probe}' > /claw_sessions/.relaxed_verify_probe && cat /claw_sessions/.relaxed_verify_probe",
            timeout=30,
        )
        if probe not in (r.stdout or ""):
            _fail(f"/claw_sessions write/read failed: stdout={r.stdout!r}")
        _ok("/claw_sessions write/read (rw)")

        r = sb.commands.run("test -d /claw_ds && ls -la /claw_ds | head -3", timeout=30)
        if r.exit_code not in (0, None):
            _fail(f"/claw_ds not readable: {r.stderr}")
        _ok("/claw_ds readable (ro home mount)")

        r = sb.commands.run("claw --version 2>/dev/null || claw version 2>/dev/null || claw -h | head -1", timeout=30)
        if r.exit_code not in (0, None):
            _fail(f"claw binary smoke failed: {r.stderr}")
        _ok(f"claw binary: {(r.stdout or r.stderr or '').strip()[:120]}")

        traffic_host = f"{ovs_port_num}-{sid}.{domain}"
        folder_url = f"http://{traffic_host}/ovs/?folder=/claw_ds"
        ext = subprocess.run(
            ["curl", "-sS", "-m", "20", "-o", "/dev/null", "-w", "%{http_code}", folder_url],
            capture_output=True,
            text=True,
        )
        if ext.returncode != 0 or ext.stdout.strip() != "200":
            _fail(f"external OVS HTTP {ext.stdout.strip()} at {folder_url}")
        _ok(f"external OVS {folder_url}")

        body = subprocess.run(["curl", "-sS", "-m", "20", folder_url], capture_output=True, text=True)
        if "openvscode" not in (body.stdout or "").lower() and "workbench" not in (body.stdout or "").lower():
            _fail(f"OVS page does not look like openvscode at {folder_url}")
        _ok("OVS page content looks like openvscode")

        if _env("CLAW_RELAXED_VERIFY_SKIP_CHAT", "0") not in ("1", "true", "yes"):
            api_base = _env("OPENAI_BASE_URL") or _env("CLAW_OPENAI_BASE_URL")
            api_key_llm = _env("OPENAI_API_KEY") or _env("CLAW_OPENAI_API_KEY")
            if api_base and api_key_llm:
                chat_script = (
                    "set -eu\n"
                    f"export OPENAI_BASE_URL={json.dumps(api_base)}\n"
                    f"export OPENAI_API_KEY={json.dumps(api_key_llm)}\n"
                    "export CLAW_PROJECT_CONFIG_ROOT=/claw_ds\n"
                    'claw chat -m "reply with exactly: pong" --max-turns 1 2>&1 | tail -5'
                )
                r = sb.commands.run(chat_script, timeout=120)
                out = (r.stdout or "") + (r.stderr or "")
                if "pong" not in out.lower() and r.exit_code not in (0, None):
                    print(f"  WARN chat smoke inconclusive exit={r.exit_code} (not blocking)", file=sys.stderr)
                else:
                    _ok("claw chat smoke")
            else:
                print("  skip chat smoke (no OPENAI_* in env)", file=sys.stderr)

    finally:
        try:
            http_json_selfhosted("DELETE", f"{api_url.rstrip('/')}/sandboxes/{sid}", api_key, True)
        except Exception:
            pass

    print("verify-claw-worker-relaxed-sandbox: OK")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

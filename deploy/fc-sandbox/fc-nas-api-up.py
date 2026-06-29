#!/usr/bin/env python3
"""Ensure FC claw-nas-api singleton sandbox exists on e2b — gateway NAS read/write service.

nas-api is started only by the claw-nas-api template startCmd (envd bootstrap on
sandbox create). This script does NOT fc_exec a second launch path — same contract
as fc-ovs-up.py / fc-tap-live-up.py. The gateway is a pure consumer: it reads the
persisted endpoint (gateway_global_settings.settings_json.fcNasApi) and never creates
the sandbox itself.

Usage (repo root, `.env` with self-hosted e2b vars):
  python3 deploy/fc-sandbox/fc-nas-api-up.py
  python3 deploy/fc-sandbox/fc-nas-api-up.py --reuse
  python3 deploy/fc-sandbox/fc-nas-api-up.py --reset
  python3 deploy/fc-sandbox/fc-nas-api-up.py --json

Author: kejiqing
"""
from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[2]
_FC_SANDBOX_DIR = Path(__file__).resolve().parent
if str(_FC_SANDBOX_DIR) not in sys.path:
    sys.path.insert(0, str(_FC_SANDBOX_DIR))
GUEST_CLAW_WS = "/claw_ws"
_FC_VENV_DEPS = ("python-dotenv", "psycopg[binary]")


def _fc_venv_dir() -> Path:
    raw = _env("CLAW_FC_VENV")
    return Path(raw) if raw else ROOT / ".venv-fc"


def _fc_python() -> Path:
    venv = _fc_venv_dir()
    py = venv / "bin" / "python3"
    if not py.is_file():
        print(f"==> create FC venv {venv} (e2b SDK + psycopg)", file=sys.stderr)
        subprocess.check_call([sys.executable, "-m", "venv", str(venv)])
        subprocess.check_call([str(venv / "bin" / "pip"), "install", "-q", *_FC_VENV_DEPS])
        return py
    try:
        subprocess.run([str(py), "-c", "import psycopg"], capture_output=True, check=True)
    except subprocess.CalledProcessError:
        subprocess.check_call([str(venv / "bin" / "pip"), "install", "-q", *_FC_VENV_DEPS])
    return py


def _ensure_fc_venv_python() -> None:
    """Re-exec under .venv-fc so psycopg is available for PG persist. Author: kejiqing

    Compare by venv prefix, not the interpreter path: the venv `python3` is a symlink
    to the base interpreter, so `resolve()` collapses both sides and would falsely
    report "already in venv" -> never re-exec -> venv site-packages missing.
    """
    fc_py = _fc_python()
    if Path(sys.prefix).resolve() != _fc_venv_dir().resolve():
        os.execv(str(fc_py), [str(fc_py), *sys.argv])


def _load_dotenv(path: Path) -> None:
    if not path.is_file():
        return
    for raw in path.read_text(encoding="utf-8").splitlines():
        line = raw.strip()
        if not line or line.startswith("#") or "=" not in line:
            continue
        key, _, val = line.partition("=")
        key = key.strip()
        val = val.strip().strip('"').strip("'")
        if key and key not in os.environ:
            os.environ[key] = val


def _env(name: str, default: str = "") -> str:
    return os.environ.get(name, default).strip()


def _is_self_hosted(api_url: str) -> bool:
    u = api_url.lower()
    return not ("aliyuncs.com" in u or "e2b.fc." in u)


def _auth_headers(api_key: str, self_hosted: bool) -> dict[str, str]:
    if self_hosted:
        return {"X-API-Key": api_key, "Content-Type": "application/json"}
    return {"Authorization": f"Bearer {api_key}", "Content-Type": "application/json"}


def _http_json(
    method: str,
    url: str,
    api_key: str,
    self_hosted: bool,
    body: dict[str, Any] | None = None,
) -> Any:
    data = None if body is None else json.dumps(body).encode("utf-8")
    req = urllib.request.Request(url, data=data, method=method, headers=_auth_headers(api_key, self_hosted))
    try:
        with urllib.request.urlopen(req, timeout=120) as resp:
            raw = resp.read().decode("utf-8")
            return json.loads(raw) if raw.strip() else {}
    except urllib.error.HTTPError as exc:
        detail = exc.read().decode("utf-8", errors="replace")
        raise RuntimeError(f"HTTP {exc.code} {method} {url}: {detail}") from exc


def _service_public_host(port: int, sandbox_id: str, domain: str) -> str:
    return f"{port}-{sandbox_id}.{domain}"


def _nas_api_port() -> int:
    try:
        return int(_env("CLAW_FC_NAS_API_PORT", "8090") or "8090")
    except ValueError:
        return 8090


def _nas_api_base_url(sandbox_id: str, domain: str, port: int, self_hosted: bool) -> str:
    host = _service_public_host(port, sandbox_id, domain)
    scheme = "http" if self_hosted else "https"
    return f"{scheme}://{host}"


def _e2b_host_mount_root(api_url: str, api_key: str, self_hosted: bool) -> str:
    """e2b host NAS bind source (hostMountRoot). Env override → /health → /mnt/nas0."""
    root = _env("CLAW_E2B_NAS_HOST_MOUNT")
    if root:
        return root
    try:
        health = _http_json("GET", f"{api_url.rstrip('/')}/health", api_key, self_hosted)
        nas = health.get("nas") or {}
        r = (nas.get("hostMountRoot") or "").strip()
        if r:
            return r
    except Exception:  # noqa: BLE001 — health is best-effort
        pass
    return "/mnt/nas0"


def _nas_config_body(api_url: str, api_key: str, self_hosted: bool) -> dict[str, Any]:
    """Host-bind-inject form (hostMountRoot + relPath) — the form e2bserver actually binds.

    nas-api MUST have /claw_ws backed by NAS (gateway writes proj layout through it),
    so unlike ovs this is required, not optional.
    """
    uid = int(_env("CLAW_WORKER_UID", "1000") or "1000")
    gid = int(_env("CLAW_WORKER_GID", "1000") or "1000")
    root = _e2b_host_mount_root(api_url, api_key, self_hosted)
    return {
        "userId": uid,
        "groupId": gid,
        "hostMountRoot": root,
        "mountPoints": [{"relPath": "", "mountDir": GUEST_CLAW_WS}],
    }


def _list_sandboxes(api_url: str, api_key: str, self_hosted: bool) -> list[dict[str, Any]]:
    rows = _http_json("GET", f"{api_url.rstrip('/')}/sandboxes", api_key, self_hosted)
    return rows if isinstance(rows, list) else []


def _sandbox_id(row: dict[str, Any]) -> str:
    for key in ("sandboxID", "sandboxId", "id"):
        val = row.get(key)
        if isinstance(val, str) and val.strip():
            return val.strip()
    return ""


def _list_nas_api_singletons(cluster_id: str, api_url: str, api_key: str, self_hosted: bool) -> list[str]:
    out: list[str] = []
    for row in _list_sandboxes(api_url, api_key, self_hosted):
        meta = row.get("metadata") or {}
        if not isinstance(meta, dict):
            continue
        if meta.get("clawRole") == "nas-api-singleton" and meta.get("clusterId") == cluster_id:
            sid = _sandbox_id(row)
            if sid:
                out.append(sid)
    return out


def _find_nas_api_singleton(cluster_id: str, api_url: str, api_key: str, self_hosted: bool) -> str | None:
    ids = _list_nas_api_singletons(cluster_id, api_url, api_key, self_hosted)
    return ids[0] if ids else None


def _reap_other_singletons(
    keep_sid: str, cluster_id: str, api_url: str, api_key: str, self_hosted: bool
) -> int:
    """Enforce one nas-api singleton per cluster: kill every match except `keep_sid`."""
    killed = 0
    for sid in _list_nas_api_singletons(cluster_id, api_url, api_key, self_hosted):
        if sid == keep_sid:
            continue
        print(f"==> reap orphan nas-api sandbox {sid}", file=sys.stderr)
        try:
            _kill_sandbox(sid, api_url, api_key, self_hosted)
            killed += 1
        except Exception as exc:  # noqa: BLE001 — best-effort cleanup
            print(f"==> warn: reap {sid} failed: {exc}", file=sys.stderr)
    return killed


def _create_nas_api_sandbox(
    *,
    api_url: str,
    api_key: str,
    self_hosted: bool,
    template: str,
    timeout_secs: int,
    cluster_id: str,
) -> tuple[str, str]:
    body: dict[str, Any] = {
        "templateID": template,
        "timeout": timeout_secs,
        "metadata": {
            "clawRole": "nas-api-singleton",
            "clusterId": cluster_id,
        },
        "nasConfig": _nas_config_body(api_url, api_key, self_hosted),
    }
    if self_hosted:
        body["secure"] = False
    parsed = _http_json("POST", f"{api_url.rstrip('/')}/sandboxes", api_key, self_hosted, body)
    sid = _sandbox_id(parsed)
    if not sid:
        raise RuntimeError(f"create sandbox: missing sandboxID in {parsed!r}")
    domain = _env("CLAW_FC_DOMAIN", _env("E2B_DOMAIN", "supone.top"))
    if not self_hosted:
        domain = (parsed.get("domain") or domain).strip() or domain
    return sid, domain


def _kill_sandbox(sandbox_id: str, api_url: str, api_key: str, self_hosted: bool) -> None:
    _http_json("DELETE", f"{api_url.rstrip('/')}/sandboxes/{sandbox_id}", api_key, self_hosted)


def _persist_fc_nas_api_to_pg(base_url: str, sandbox_id: str) -> None:
    from fc_pg_settings import merge_settings_json_key

    now_ms = int(time.time() * 1000)
    merge_settings_json_key(
        "fcNasApi",
        {"baseUrl": base_url, "sandboxId": sandbox_id, "updatedAtMs": now_ms},
        now_ms=now_ms,
    )


def _healthz_ok(base_url: str, api_key: str, self_hosted: bool) -> bool:
    try:
        data = _http_json("GET", f"{base_url.rstrip('/')}/healthz", api_key, self_hosted)
        return bool(isinstance(data, dict) and data.get("ok"))
    except Exception:  # noqa: BLE001 — 503/conn-refused → not ready yet
        return False


def _wait_healthz(base_url: str, api_key: str, self_hosted: bool, *, max_attempts: int = 60, sleep_sec: int = 2) -> bool:
    """Wait for template startCmd to bring up /healthz (no fc_exec launch)."""
    for i in range(1, max_attempts + 1):
        if _healthz_ok(base_url, api_key, self_hosted):
            print(f"==> nas-api healthz ready ({base_url}, attempt {i}/{max_attempts})", file=sys.stderr)
            return True
        print(f"==> waiting nas-api healthz ({i}/{max_attempts}) …", file=sys.stderr)
        time.sleep(sleep_sec)
    return False


def main() -> int:
    parser = argparse.ArgumentParser(description="Start e2b FC claw-nas-api singleton (gateway NAS service)")
    parser.add_argument("--reuse", action="store_true", help="reuse existing nas-api-singleton sandbox")
    parser.add_argument(
        "--reset",
        action="store_true",
        help="kill existing nas-api-singleton, create fresh sandbox, write PG",
    )
    parser.add_argument("--kill", metavar="SANDBOX_ID", help="kill sandbox and exit")
    parser.add_argument("--json", action="store_true", help="print JSON only")
    parser.add_argument("--no-persist", action="store_true", help="skip writing fcNasApi to PG")
    args = parser.parse_args()

    _load_dotenv(ROOT / ".env")
    _ensure_fc_venv_python()

    api_key = _env("CLAW_FC_API_KEY") or _env("E2B_API_KEY") or _env("ALIYUN_E2B_TOKEN")
    if not api_key:
        print("error: set CLAW_FC_API_KEY in .env", file=sys.stderr)
        return 1

    api_url = _env("CLAW_FC_API_URL") or _env("E2B_API_URL") or "http://10.8.0.1:3000"
    fc_domain = _env("CLAW_FC_DOMAIN") or _env("E2B_DOMAIN") or "supone.top"
    cluster_id = _env("CLAW_CLUSTER_ID") or "default"
    template = _env("CLAW_FC_NAS_API_TEMPLATE") or "claw-nas-api"
    timeout_secs = int(_env("CLAW_FC_SANDBOX_TIMEOUT_SECS", "3600") or "3600")
    port = _nas_api_port()
    self_hosted = _is_self_hosted(api_url)

    if args.kill:
        _kill_sandbox(args.kill.strip(), api_url, api_key, self_hosted)
        print(f"killed sandbox {args.kill}")
        return 0

    sandbox_id: str | None = None
    domain = fc_domain

    if args.reset:
        for existing in _list_nas_api_singletons(cluster_id, api_url, api_key, self_hosted):
            print(f"==> reset: kill nas-api sandbox {existing}", file=sys.stderr)
            _kill_sandbox(existing, api_url, api_key, self_hosted)
    elif args.reuse:
        sandbox_id = _find_nas_api_singleton(cluster_id, api_url, api_key, self_hosted)
        if sandbox_id:
            print(f"==> reuse nas-api sandbox {sandbox_id}", file=sys.stderr)

    if not sandbox_id:
        print(f"==> create nas-api sandbox (template={template}, cluster={cluster_id})", file=sys.stderr)
        sandbox_id, domain = _create_nas_api_sandbox(
            api_url=api_url,
            api_key=api_key,
            self_hosted=self_hosted,
            template=template,
            timeout_secs=timeout_secs,
            cluster_id=cluster_id,
        )
        print(f"==> sandbox_id={sandbox_id}", file=sys.stderr)

    base_url = _nas_api_base_url(sandbox_id, domain, port, self_hosted)
    print("==> wait for nas-api healthz (template startCmd; no fc_exec launch) …", file=sys.stderr)
    if not _wait_healthz(base_url, api_key, self_hosted):
        raise RuntimeError(
            f"nas-api healthz not reachable at {base_url}/healthz — "
            "claw-nas-api template startCmd should start the service on sandbox create; "
            "rebuild template (build-claw-nas-api-selfhosted.py) or recreate sandbox (--reset)"
        )

    # Enforce single nas-api singleton per cluster: reap any other matches
    # (orphans from earlier debug runs or the removed gateway self-create path).
    reaped = _reap_other_singletons(sandbox_id, cluster_id, api_url, api_key, self_hosted)
    if reaped:
        print(f"==> reaped {reaped} orphan nas-api sandbox(es)", file=sys.stderr)

    result = {
        "sandboxId": sandbox_id,
        "clusterId": cluster_id,
        "baseUrl": base_url,
        "healthzUrl": f"{base_url}/healthz",
        "servicePort": str(port),
        "healthy": True,
        "reapedOrphans": reaped,
    }

    if not args.no_persist:
        print("==> persist fcNasApi to PG …", file=sys.stderr)
        _persist_fc_nas_api_to_pg(base_url, sandbox_id)

    if args.json:
        print(json.dumps(result, indent=2, ensure_ascii=False))
    else:
        print()
        print("FC claw-nas-api singleton (gateway NAS read/write service)")
        print(f"  sandboxId:  {sandbox_id}")
        print(f"  baseUrl:    {base_url}")
        print(f"  healthz:    {base_url}/healthz")
        print()
        print(f"# Verify: curl -fsS {base_url}/healthz")
        print(f"# Stat:   curl -fsS {base_url}/v1/stat/proj_1/workers")
        print("nas-api healthz: OK")

    return 0


if __name__ == "__main__":
    raise SystemExit(main())

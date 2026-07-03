#!/usr/bin/env python3
"""Ensure e2b observe-singleton sandbox on e2b — direct Live traffic URL, no gateway proxy.

claude-tap Live is started only by the claw-observe template startCmd (Panel envd bootstrap
on sandbox create). This script does not fc_exec a second launch path.

Usage (from repo root, after `.env` with self-hosted e2b vars):
  python3 deploy/e2b/e2b-tap-live-up.py
  python3 deploy/e2b/e2b-tap-live-up.py --reuse
  python3 deploy/e2b/e2b-tap-live-up.py --reset
  python3 deploy/e2b/e2b-tap-live-up.py --json

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
    raw = _env("CLAW_E2B_VENV")
    return Path(raw) if raw else ROOT / ".venv-fc"


def _fc_python() -> Path:
    """Python with psycopg for PG persist (auto-create repo .venv-fc when missing). Author: kejiqing"""
    venv = _fc_venv_dir()
    py = venv / "bin" / "python3"
    if not py.is_file():
        print(f"==> create e2b venv {venv} (psycopg for PG persist)", file=sys.stderr)
        subprocess.check_call([sys.executable, "-m", "venv", str(venv)])
        subprocess.check_call([str(venv / "bin" / "pip"), "install", "-q", *_FC_VENV_DEPS])
        return py
    try:
        subprocess.run(
            [str(py), "-c", "import psycopg"],
            capture_output=True,
            check=True,
        )
    except subprocess.CalledProcessError:
        print(f"==> install e2b venv deps in {venv}", file=sys.stderr)
        subprocess.check_call([str(venv / "bin" / "pip"), "install", "-q", *_FC_VENV_DEPS])
    return py


def _ensure_fc_venv_python() -> None:
    """Re-exec under .venv-fc when psycopg missing; skip when system python already has deps.

    Gateway container installs psycopg in the image — no writable venv under /app. Author: kejiqing
    """
    try:
        import psycopg  # noqa: F401
        return
    except ImportError:
        pass
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


def _nas_server_addr(server: str, export: str, rel_path: str) -> str:
    host = server.strip().rstrip("/")
    rel = rel_path.lstrip("/")
    export = export.strip()
    if not export or export == "/":
        return f"{host}:/{rel}"
    export = export.lstrip("/").rstrip("/")
    return f"{host}:/{export}/{rel}"


def _nas_config_body() -> dict[str, Any] | None:
    server = _env("CLAW_E2B_NAS_SERVER") or _env("NAS_BASE_URL")
    if not server:
        return None
    export = _env("CLAW_E2B_NAS_EXPORT") or "/"
    uid = int(_env("CLAW_WORKER_UID", "1000") or "1000")
    gid = int(_env("CLAW_WORKER_GID", "1000") or "1000")
    return {
        "userId": uid,
        "groupId": gid,
        "mountPoints": [
            {"serverAddr": _nas_server_addr(server, export, ""), "mountDir": GUEST_CLAW_WS},
        ],
    }


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


def _traffic_port() -> int:
    try:
        return int(_env("CLAW_E2B_TRAFFIC_PORT", "3001") or "3001")
    except ValueError:
        return 3001


def _observe_live_port() -> int:
    try:
        return int(_env("CLAW_E2B_OBSERVE_LIVE_PORT", "3000") or "3000")
    except ValueError:
        return 3000


def _internal_live_base(sandbox_id: str, domain: str, live_port: int) -> str:
    host = _service_public_host(live_port, sandbox_id, domain)
    scheme = "http" if _is_self_hosted(_env("CLAW_E2B_API_URL", "http://10.8.0.1:3000")) else "https"
    return f"{scheme}://{host}"


def _browser_urls(sandbox_id: str, domain: str, live_port: int) -> dict[str, str]:
    """Shareable browser URL: http://{port}-{sandboxId}.{domain}/"""
    direct = _internal_live_base(sandbox_id, domain, live_port)
    trail = direct if direct.endswith("/") else f"{direct}/"
    return {
        "liveBaseUrl": direct,
        "liveSessionUrlTemplate": f"{trail}?session={{sessionId}}",
        "sandboxId": sandbox_id,
        "servicePort": str(live_port),
    }


def _database_url() -> str:
    for key in ("CLAW_E2B_WORKER_DATABASE_URL", "CLAW_GATEWAY_DATABASE_URL"):
        val = _env(key)
        if val:
            return val
    raise RuntimeError("set CLAW_E2B_WORKER_DATABASE_URL or CLAW_GATEWAY_DATABASE_URL in .env")


def _observe_template_id() -> str:
    """PG e2bObserve.templateId (last build) → env → alias default."""
    try:
        from e2b_pg_settings import load_settings_json_key

        row = load_settings_json_key("e2bObserve")
        tid = row.get("templateId") if isinstance(row, dict) else None
        if isinstance(tid, str) and tid.strip():
            return tid.strip()
    except Exception as exc:  # noqa: BLE001
        print(f"warn: load e2bObserve.templateId from PG: {exc}", file=sys.stderr)
    return _env("CLAW_E2B_OBSERVE_TEMPLATE") or "claw-observe"


def _list_sandboxes(api_url: str, api_key: str, self_hosted: bool) -> list[dict[str, Any]]:
    rows = _http_json("GET", f"{api_url.rstrip('/')}/sandboxes", api_key, self_hosted)
    if isinstance(rows, list):
        return rows
    return []


def _sandbox_id(row: dict[str, Any]) -> str:
    for key in ("sandboxID", "sandboxId", "id"):
        val = row.get(key)
        if isinstance(val, str) and val.strip():
            return val.strip()
    return ""


def _find_observe_singleton(cluster_id: str, api_url: str, api_key: str, self_hosted: bool) -> str | None:
    for row in _list_sandboxes(api_url, api_key, self_hosted):
        meta = row.get("metadata") or {}
        if not isinstance(meta, dict):
            continue
        if meta.get("clawRole") == "observe-singleton" and meta.get("clusterId") == cluster_id:
            sid = _sandbox_id(row)
            if sid:
                return sid
    return None


def _create_observe_sandbox(
    *,
    api_url: str,
    api_key: str,
    self_hosted: bool,
    template: str,
    timeout_secs: int,
    cluster_id: str,
    db_url: str,
) -> tuple[str, str]:
    body: dict[str, Any] = {
        "templateID": template,
        "timeout": timeout_secs,
        "metadata": {
            "clawRole": "observe-singleton",
            "clusterId": cluster_id,
        },
        "envVars": {
            "CLAW_CLUSTER_ID": cluster_id,
            "CLAW_GATEWAY_DATABASE_URL": db_url,
        },
    }
    nas = _nas_config_body()
    if nas:
        body["nasConfig"] = nas
    if self_hosted:
        body["secure"] = False
    parsed = _http_json("POST", f"{api_url.rstrip('/')}/sandboxes", api_key, self_hosted, body)
    sid = _sandbox_id(parsed)
    if not sid:
        raise RuntimeError(f"create sandbox: missing sandboxID in {parsed!r}")
    domain = _env("CLAW_E2B_DOMAIN", _env("E2B_DOMAIN", "supone.top"))
    if not self_hosted:
        domain = (parsed.get("domain") or domain).strip() or domain
    return sid, domain


def _kill_sandbox(sandbox_id: str, api_url: str, api_key: str, self_hosted: bool) -> None:
    _http_json("DELETE", f"{api_url.rstrip('/')}/sandboxes/{sandbox_id}", api_key, self_hosted)


def _apply_sandbox_timeout(
    sandbox_id: str,
    *,
    api_url: str,
    api_key: str,
    self_hosted: bool,
    timeout_secs: int,
) -> None:
    """e2b create may leave template default TTL (~300s); POST /timeout applies CLAW_E2B_SANDBOX_TIMEOUT_SECS."""
    _http_json(
        "POST",
        f"{api_url.rstrip('/')}/sandboxes/{sandbox_id}/timeout",
        api_key,
        self_hosted,
        {"timeout": timeout_secs},
    )


def _proxy_base_url(sandbox_id: str, domain: str, proxy_port: int = 8080) -> str:
    host = _service_public_host(proxy_port, sandbox_id, domain)
    scheme = "http" if _is_self_hosted(_env("CLAW_E2B_API_URL", "http://10.8.0.1:3000")) else "https"
    return f"{scheme}://{host}"


def _persist_observe_urls_to_pg(urls: dict[str, str], domain: str, live_port: int) -> None:
    """Write observe tap URLs into gateway_global_settings.settings_json.clawTap. Author: kejiqing"""
    from e2b_pg_settings import merge_settings_json_key

    sandbox_id = urls["sandboxId"]
    proxy_port = 8080
    proxy_host = _service_public_host(proxy_port, sandbox_id, domain)
    now_ms = int(time.time() * 1000)
    patch = {
        "mode": "remote",
        "host": proxy_host,
        "proxyPort": proxy_port,
        "livePort": live_port,
        "updatedAtMs": now_ms,
        "liveBaseUrl": urls["liveBaseUrl"],
        "liveSessionUrlTemplate": urls["liveSessionUrlTemplate"],
        "proxyBaseUrl": _proxy_base_url(sandbox_id, domain, proxy_port),
        "e2bObserveSandboxId": sandbox_id,
    }
    merge_settings_json_key("clawTap", patch, now_ms=now_ms)


def _verify_traffic(live_base_url: str) -> bool:
    proc = subprocess.run(
        [
            "curl",
            "-sS",
            "--connect-timeout",
            "5",
            "--max-time",
            "15",
            live_base_url,
            "-o",
            "/dev/null",
            "-w",
            "%{http_code}",
        ],
        capture_output=True,
        text=True,
        check=False,
    )
    code = (proc.stdout or "").strip()
    # curl 18 = partial transfer (large Live HTML via e2b nginx); HTTP 2xx still OK.
    ok_exit = proc.returncode in (0, 18)
    return ok_exit and code.startswith("2")


def _wait_live_traffic(live_base_url: str, *, max_attempts: int = 60, sleep_sec: int = 2) -> bool:
    """Wait for Panel template startCmd to bring up Live traffic."""
    for i in range(1, max_attempts + 1):
        if _verify_traffic(live_base_url):
            print(f"==> observe Live traffic ready ({live_base_url}, attempt {i}/{max_attempts})", file=sys.stderr)
            return True
        print(f"==> waiting observe Live traffic ({i}/{max_attempts}) …", file=sys.stderr)
        time.sleep(sleep_sec)
    return False


def main() -> int:
    parser = argparse.ArgumentParser(description="Ensure e2b observe-singleton (claude-tap Live)")
    parser.add_argument("--reuse", action="store_true", help="reuse existing observe-singleton sandbox")
    parser.add_argument(
        "--reset",
        action="store_true",
        help="kill existing observe-singleton, create fresh sandbox, write PG",
    )
    parser.add_argument("--kill", metavar="SANDBOX_ID", help="kill sandbox and exit")
    parser.add_argument("--json", action="store_true", help="print JSON only")
    parser.add_argument(
        "--no-persist",
        action="store_true",
        help="skip writing liveBaseUrl to gateway_global_settings (default: persist on success)",
    )
    args = parser.parse_args()

    _load_dotenv(ROOT / ".env")
    _ensure_fc_venv_python()

    api_key = _env("CLAW_E2B_API_KEY") or _env("E2B_API_KEY") or _env("ALIYUN_E2B_TOKEN")
    if not api_key:
        print("error: set CLAW_E2B_API_KEY (or E2B_API_KEY) in .env", file=sys.stderr)
        return 1

    api_url = _env("CLAW_E2B_API_URL") or _env("E2B_API_URL") or "http://10.8.0.1:3000"
    fc_domain = _env("CLAW_E2B_DOMAIN") or _env("E2B_DOMAIN") or "supone.top"
    cluster_id = _env("CLAW_CLUSTER_ID") or "default"
    template = _observe_template_id()
    timeout_secs = int(_env("CLAW_E2B_SANDBOX_TIMEOUT_SECS", "3600") or "3600")
    live_port = _observe_live_port()
    self_hosted = _is_self_hosted(api_url)
    from e2b_pg_settings import sandbox_database_url

    db_url = _database_url()
    sandbox_db_url = sandbox_database_url()
    if sandbox_db_url != db_url:
        print(f"==> sandbox PG URL: {sandbox_db_url!r} (not 127.0.0.1 — e2b host must reach Mac PG)", file=sys.stderr)

    if args.kill:
        _kill_sandbox(args.kill.strip(), api_url, api_key, self_hosted)
        print(f"killed sandbox {args.kill}")
        return 0

    sandbox_id: str | None = None
    domain = fc_domain

    if args.reset:
        existing = _find_observe_singleton(cluster_id, api_url, api_key, self_hosted)
        if existing:
            print(f"==> reset: kill observe sandbox {existing}", file=sys.stderr)
            _kill_sandbox(existing, api_url, api_key, self_hosted)
    elif args.reuse:
        sandbox_id = _find_observe_singleton(cluster_id, api_url, api_key, self_hosted)
        if sandbox_id:
            print(f"==> reuse observe sandbox {sandbox_id}", file=sys.stderr)

    if not sandbox_id:
        print(f"==> create observe sandbox (template={template}, cluster={cluster_id})", file=sys.stderr)
        sandbox_id, domain = _create_observe_sandbox(
            api_url=api_url,
            api_key=api_key,
            self_hosted=self_hosted,
            template=template,
            timeout_secs=timeout_secs,
            cluster_id=cluster_id,
            db_url=sandbox_db_url,
        )
        print(f"==> sandbox_id={sandbox_id}", file=sys.stderr)
        print(f"==> apply sandbox TTL {timeout_secs}s (POST /timeout)", file=sys.stderr)
        _apply_sandbox_timeout(
            sandbox_id,
            api_url=api_url,
            api_key=api_key,
            self_hosted=self_hosted,
            timeout_secs=timeout_secs,
        )

    urls = _browser_urls(sandbox_id, domain, live_port)
    urls["clusterId"] = cluster_id
    urls["internalLiveBaseUrl"] = _internal_live_base(sandbox_id, domain, live_port)

    print(
        "==> wait for observe Live traffic (template startCmd via Panel; no fc_exec launch) …",
        file=sys.stderr,
    )
    if not _wait_live_traffic(urls["liveBaseUrl"]):
        raise RuntimeError(
            f"observe Live traffic not reachable at {urls['liveBaseUrl']} — "
            "claw-observe template startCmd should start claude-tap on sandbox create; "
            "rebuild template (build-claw-observe-selfhosted.py) or recreate sandbox (--reset)"
        )

    ok = _verify_traffic(urls["liveBaseUrl"])
    urls["trafficReachable"] = ok

    if not args.no_persist:
        print("==> persist observe tap URLs to PG …", file=sys.stderr)
        _persist_observe_urls_to_pg(urls, domain, live_port)

    if args.json:
        print(json.dumps(urls, indent=2, ensure_ascii=False))
    else:
        print()
        print("claude-tap Live (e2b Host traffic — no gateway proxy)")
        print(f"  sandboxId:              {sandbox_id}")
        print(f"  liveBaseUrl:            {urls['liveBaseUrl']}")
        print(f"  liveSessionUrlTemplate: {urls['liveSessionUrlTemplate']}")
        print()
        print("# Verify:")
        print(f"  curl -fsS {urls['liveBaseUrl']}")
        if ok:
            print()
            print(f"traffic check: OK ({urls['liveBaseUrl']})")
        else:
            print()
            print("traffic check: FAILED — check DNS/nginx Host routing for", fc_domain, file=sys.stderr)
            return 2

    return 0 if ok else 2


if __name__ == "__main__":
    raise SystemExit(main())

#!/usr/bin/env python3
"""Start FC OVS singleton (openvscode-server) on e2b — direct traffic URL, no gateway.

Usage (repo root, `.env` with self-hosted e2b vars):
  python3 deploy/fc-sandbox/fc-ovs-up.py
  python3 deploy/fc-sandbox/fc-ovs-up.py --reuse
  python3 deploy/fc-sandbox/fc-ovs-up.py --reset
  python3 deploy/fc-sandbox/fc-ovs-up.py --json

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
EXEC_HELPER = ROOT / "deploy/fc-sandbox/fc_exec.py"
GUEST_CLAW_WS = "/claw_ws"
_FC_VENV_DEPS = ("e2b==2.26.0", "e2b-code-interpreter", "python-dotenv", "psycopg[binary]")


def _fc_venv_dir() -> Path:
    raw = _env("CLAW_FC_VENV")
    return Path(raw) if raw else ROOT / ".venv-fc"


def _fc_python() -> Path:
    venv = _fc_venv_dir()
    py = venv / "bin" / "python3"
    if not py.is_file():
        print(f"==> create FC venv {venv} (e2b SDK for fc_exec)", file=sys.stderr)
        subprocess.check_call([sys.executable, "-m", "venv", str(venv)])
        subprocess.check_call([str(venv / "bin" / "pip"), "install", "-q", *_FC_VENV_DEPS])
        return py
    try:
        subprocess.run(
            [str(py), "-c", "import e2b_code_interpreter; import psycopg"],
            capture_output=True,
            check=True,
        )
    except subprocess.CalledProcessError:
        subprocess.check_call([str(venv / "bin" / "pip"), "install", "-q", *_FC_VENV_DEPS])
    return py


def _ensure_fc_venv_python() -> None:
    """Re-exec under .venv-fc so psycopg + e2b share one interpreter. Author: kejiqing"""
    fc_py = _fc_python()
    if Path(sys.executable).resolve() != fc_py.resolve():
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
    server = _env("CLAW_FC_NAS_SERVER") or _env("NAS_BASE_URL")
    if not server:
        return None
    export = _env("CLAW_FC_NAS_EXPORT") or "/"
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


def _ovs_port() -> int:
    try:
        return int(_env("CLAW_FC_OVS_PORT", "3000") or "3000")
    except ValueError:
        return 3000


def _ovs_base_url(sandbox_id: str, domain: str, ovs_port: int) -> str:
    host = _service_public_host(ovs_port, sandbox_id, domain)
    scheme = "http" if _is_self_hosted(_env("CLAW_FC_API_URL", "http://10.8.0.9:3000")) else "https"
    return f"{scheme}://{host}/ovs"


def _workspace_folder_url(base_url: str, proj_id: int) -> str:
    base = base_url.rstrip("/")
    return f"{base}?folder={GUEST_CLAW_WS}/proj_{proj_id}/home"


def _start_ovs_script(ovs_port: int) -> str:
    return f"""set -e
OVS_BIN="/home/.openvscode-server/bin/openvscode-server"
if [ ! -x "$OVS_BIN" ]; then
  echo "fc ovs: openvscode-server not found (rebuild claw-ovs template)" >&2
  exit 127
fi
OVS_LOG="{GUEST_CLAW_WS}/.claw-ovs.log"
OVS_PID="{GUEST_CLAW_WS}/.claw-ovs.pid"
if [ -f "$OVS_PID" ] && kill -0 "$(cat "$OVS_PID")" 2>/dev/null; then
  if curl -fsS --connect-timeout 2 "http://127.0.0.1:{ovs_port}/ovs/" >/dev/null 2>&1; then
    exit 0
  fi
  kill "$(cat "$OVS_PID")" 2>/dev/null || true
  rm -f "$OVS_PID"
fi
export HOME=/opt/claw-ovs/home
mkdir -p /opt/claw-ovs/home /opt/claw-extensions /opt/claw-ovs/server-data/data/logs /opt/claw-ovs/server-data/data/Machine {GUEST_CLAW_WS}
nohup "$OVS_BIN" \\
  --host=0.0.0.0 --port={ovs_port} \\
  --without-connection-token \\
  --server-base-path=/ovs \\
  --extensions-dir=/opt/claw-extensions \\
  --server-data-dir=/opt/claw-ovs/server-data \\
  --enable-proposed-api=claw.claw-vscode,claw.ovs-chat-demo \\
  >"$OVS_LOG" 2>&1 &
echo $! >"$OVS_PID"
for _ in $(seq 1 30); do
  if curl -fsS --connect-timeout 2 "http://127.0.0.1:{ovs_port}/ovs/" >/dev/null 2>&1; then
    exit 0
  fi
  sleep 1
done
echo "fc ovs: openvscode /ovs/ timeout (see $OVS_LOG)" >&2
exit 1
"""


def _run_fc_exec(
    *,
    sandbox_id: str,
    script: str,
    api_key: str,
    api_url: str,
    sandbox_url: str,
    domain: str,
    self_hosted: bool,
    timeout: int = 180,
    sandbox_timeout: int = 0,
) -> None:
    if not EXEC_HELPER.is_file():
        raise RuntimeError(f"missing {EXEC_HELPER}")
    payload = {
        "op": "run_sh",
        "api_key": api_key,
        "domain": domain,
        "api_url": api_url,
        "sandbox_url": sandbox_url or None,
        "sandbox_id": sandbox_id,
        "script": script,
        "self_hosted": self_hosted,
        "timeout": timeout,
        "sandbox_timeout": sandbox_timeout,
    }
    proc = subprocess.run(
        [str(_fc_python()), str(EXEC_HELPER)],
        input=json.dumps(payload).encode("utf-8"),
        capture_output=True,
        check=False,
    )
    if proc.returncode != 0:
        err = proc.stderr.decode("utf-8", errors="replace").strip()
        out = proc.stdout.decode("utf-8", errors="replace").strip()
        raise RuntimeError(err or out or f"fc_exec exit {proc.returncode}")


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


def _find_ovs_singleton(cluster_id: str, api_url: str, api_key: str, self_hosted: bool) -> str | None:
    for row in _list_sandboxes(api_url, api_key, self_hosted):
        meta = row.get("metadata") or {}
        if not isinstance(meta, dict):
            continue
        if meta.get("clawRole") == "ovs-singleton" and meta.get("clusterId") == cluster_id:
            sid = _sandbox_id(row)
            if sid:
                return sid
    return None


def _create_ovs_sandbox(
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
            "clawRole": "ovs-singleton",
            "clusterId": cluster_id,
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
    domain = _env("CLAW_FC_DOMAIN", _env("E2B_DOMAIN", "supone.top"))
    if not self_hosted:
        domain = (parsed.get("domain") or domain).strip() or domain
    return sid, domain


def _kill_sandbox(sandbox_id: str, api_url: str, api_key: str, self_hosted: bool) -> None:
    _http_json("DELETE", f"{api_url.rstrip('/')}/sandboxes/{sandbox_id}", api_key, self_hosted)


def _persist_fc_ovs_to_pg(urls: dict[str, str]) -> None:
    from fc_pg_settings import merge_settings_json_key

    now_ms = int(time.time() * 1000)
    patch = {
        "baseUrl": urls["ovsUrl"],
        "sandboxId": urls["sandboxId"],
        "updatedAtMs": now_ms,
    }
    merge_settings_json_key("fcOvs", patch, now_ms=now_ms)


def _verify_traffic(ovs_url: str) -> bool:
    check = ovs_url if ovs_url.endswith("/") else f"{ovs_url}/"
    proc = subprocess.run(
        [
            "curl",
            "-sS",
            "--connect-timeout",
            "5",
            "--max-time",
            "20",
            check,
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
    return proc.returncode in (0, 18) and code.startswith("2")


def main() -> int:
    parser = argparse.ArgumentParser(description="Start e2b FC OVS singleton (openvscode-server)")
    parser.add_argument("--reuse", action="store_true", help="reuse existing ovs-singleton sandbox")
    parser.add_argument(
        "--reset",
        action="store_true",
        help="kill existing ovs-singleton, create fresh sandbox, start OVS, write PG",
    )
    parser.add_argument("--kill", metavar="SANDBOX_ID", help="kill sandbox and exit")
    parser.add_argument("--json", action="store_true", help="print JSON only")
    parser.add_argument("--no-persist", action="store_true", help="skip writing fcOvs to PG")
    parser.add_argument(
        "--proj-id",
        type=int,
        default=int(_env("CLAW_FC_E2E_PROJ_ID", "3") or "3"),
        help="sample proj for ovsFolderUrl in output",
    )
    args = parser.parse_args()

    _load_dotenv(ROOT / ".env")
    _ensure_fc_venv_python()

    api_key = _env("CLAW_FC_API_KEY") or _env("E2B_API_KEY") or _env("ALIYUN_E2B_TOKEN")
    if not api_key:
        print("error: set CLAW_FC_API_KEY in .env", file=sys.stderr)
        return 1

    api_url = _env("CLAW_FC_API_URL") or _env("E2B_API_URL") or "http://10.8.0.9:3000"
    sandbox_url = _env("CLAW_E2B_SANDBOX_URL") or _env("E2B_SANDBOX_URL")
    fc_domain = _env("CLAW_FC_DOMAIN") or _env("E2B_DOMAIN") or "supone.top"
    cluster_id = _env("CLAW_CLUSTER_ID") or "default"
    template = _env("CLAW_FC_OVS_TEMPLATE") or "claw-ovs"
    timeout_secs = int(_env("CLAW_FC_SANDBOX_TIMEOUT_SECS", "3600") or "3600")
    ovs_port = _ovs_port()
    self_hosted = _is_self_hosted(api_url)

    if args.kill:
        _kill_sandbox(args.kill.strip(), api_url, api_key, self_hosted)
        print(f"killed sandbox {args.kill}")
        return 0

    sandbox_id: str | None = None
    domain = fc_domain

    if args.reset:
        existing = _find_ovs_singleton(cluster_id, api_url, api_key, self_hosted)
        if existing:
            print(f"==> reset: kill ovs sandbox {existing}", file=sys.stderr)
            _kill_sandbox(existing, api_url, api_key, self_hosted)
    elif args.reuse:
        sandbox_id = _find_ovs_singleton(cluster_id, api_url, api_key, self_hosted)
        if sandbox_id:
            print(f"==> reuse ovs sandbox {sandbox_id}", file=sys.stderr)

    if not sandbox_id:
        print(f"==> create ovs sandbox (template={template}, cluster={cluster_id})", file=sys.stderr)
        sandbox_id, domain = _create_ovs_sandbox(
            api_url=api_url,
            api_key=api_key,
            self_hosted=self_hosted,
            template=template,
            timeout_secs=timeout_secs,
            cluster_id=cluster_id,
        )
        print(f"==> sandbox_id={sandbox_id}", file=sys.stderr)

    print("==> start openvscode-server inside sandbox …", file=sys.stderr)
    _run_fc_exec(
        sandbox_id=sandbox_id,
        script=_start_ovs_script(ovs_port),
        api_key=api_key,
        api_url=api_url,
        sandbox_url=sandbox_url,
        domain=domain,
        self_hosted=self_hosted,
        sandbox_timeout=timeout_secs,
    )

    ovs_url = _ovs_base_url(sandbox_id, domain, ovs_port)
    folder_url = _workspace_folder_url(ovs_url, args.proj_id)
    result = {
        "sandboxId": sandbox_id,
        "clusterId": cluster_id,
        "ovsUrl": ovs_url,
        "ovsFolderUrl": folder_url,
        "servicePort": str(ovs_port),
        "trafficReachable": _verify_traffic(ovs_url),
    }

    if not args.no_persist:
        print("==> persist fcOvs to PG …", file=sys.stderr)
        _persist_fc_ovs_to_pg(result)

    if args.json:
        print(json.dumps(result, indent=2, ensure_ascii=False))
    else:
        print()
        print("FC OVS singleton (e2b Host traffic — no gateway proxy)")
        print(f"  sandboxId:     {sandbox_id}")
        print(f"  ovsUrl:        {ovs_url}")
        print(f"  ovsFolderUrl:  {folder_url}")
        print()
        print(f"# Verify: curl -fsS {ovs_url}/")
        if result["trafficReachable"]:
            print(f"traffic check: OK ({ovs_url})")
        else:
            print("traffic check: FAILED", file=sys.stderr)
            return 2

    return 0 if result["trafficReachable"] else 2


if __name__ == "__main__":
    raise SystemExit(main())

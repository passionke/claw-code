#!/usr/bin/env python3
"""Runtime install claw-vscode into FC OVS singleton (openvscode --install-extension).

VSIX is copied into the sandbox via base64 (no NAS write required).
Optionally also stages a copy on NAS when the mount is writable.

Usage (repo root, .env loaded):
  python3 deploy/fc-sandbox/fc-ovs-install-claw-vscode.py
  python3 deploy/fc-sandbox/fc-ovs-install-claw-vscode.py --reuse
  python3 deploy/fc-sandbox/fc-ovs-install-claw-vscode.py --json

Author: kejiqing
"""
from __future__ import annotations

import argparse
import base64
import json
import os
import shutil
import subprocess
import sys
import urllib.error
import urllib.request
from pathlib import Path
from typing import Any
from urllib.parse import urlparse

ROOT = Path(__file__).resolve().parents[2]
EXEC_HELPER = ROOT / "deploy/fc-sandbox/fc_exec.py"
GUEST_CLAW_WS = "/claw_ws"
_FC_VENV_DEPS = ("e2b==2.26.0", "e2b-code-interpreter", "python-dotenv")


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
            [str(py), "-c", "import e2b_code_interpreter"],
            capture_output=True,
            check=True,
        )
    except subprocess.CalledProcessError:
        subprocess.check_call([str(venv / "bin" / "pip"), "install", "-q", *_FC_VENV_DEPS])
    return py


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


def _sandbox_id(row: dict[str, Any]) -> str:
    for key in ("sandboxID", "sandboxId", "id"):
        val = row.get(key)
        if isinstance(val, str) and val.strip():
            return val.strip()
    return ""


def _list_sandboxes(api_url: str, api_key: str, self_hosted: bool) -> list[dict[str, Any]]:
    rows = _http_json("GET", f"{api_url.rstrip('/')}/sandboxes", api_key, self_hosted)
    if isinstance(rows, list):
        return rows
    return []


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


def _ovs_port() -> int:
    try:
        return int(_env("CLAW_FC_OVS_PORT", "3000") or "3000")
    except ValueError:
        return 3000


def _ext_version() -> str:
    pkg = ROOT / "extensions/claw-vscode/package.json"
    return json.loads(pkg.read_text(encoding="utf-8"))["version"]


def _nas_tools_dest() -> Path:
    tools_rel = _env("CLAW_FC_NAS_TOOLS_REL", ".claw-fc-tools") or ".claw-fc-tools"
    if _env("CLAW_NAS_HOST_MOUNT"):
        nas_root = Path(_env("CLAW_NAS_HOST_MOUNT"))
    elif _env("CLAW_POOL_WORK_ROOT_BIND_SRC"):
        nas_root = Path(_env("CLAW_POOL_WORK_ROOT_BIND_SRC"))
    else:
        nas_root = ROOT / "deploy/stack/claw-workspace"
    return nas_root / tools_rel


def _package_vsix_host() -> Path:
    subprocess.run(
        [str(ROOT / "deploy/stack/lib/package-claw-vscode-vsix.sh")],
        cwd=ROOT,
        check=True,
    )
    ext_ver = _ext_version()
    vsix_host = ROOT / f"deploy/stack/claw.claw-vscode-{ext_ver}.vsix"
    if not vsix_host.is_file() or vsix_host.stat().st_size < 1024:
        raise RuntimeError(f"invalid VSIX: {vsix_host}")
    return vsix_host


def _stage_vsix_on_nas_optional(vsix_host: Path) -> None:
    """Best-effort NAS mirror for worker bootstrap; runtime install does not depend on it."""
    dest_dir = _nas_tools_dest()
    dest = dest_dir / "claw-vscode.vsix"
    try:
        dest_dir.mkdir(parents=True, exist_ok=True)
        shutil.copy2(vsix_host, dest)
        dest.chmod(0o644)
        print(f"==> NAS VSIX mirror: {dest} ({dest.stat().st_size} bytes)", file=sys.stderr)
    except (OSError, subprocess.CalledProcessError) as exc:
        print(f"==> skip NAS VSIX mirror ({exc}); using in-sandbox base64 inject", file=sys.stderr)


def _resolve_fc_ovs_gateway_host(gateway_port: int) -> str:
    """Host:port reachable from e2b OVS sandbox (Remote EH agent WS), not gateway-rs."""
    explicit = _env("CLAW_FC_OVS_GATEWAY_HOST")
    if explicit:
        return explicit if ":" in explicit else f"{explicit}:{gateway_port}"
    advertise = _env("CLAW_POOL_ADVERTISE_HOST") or _env("CLAW_FC_GATEWAY_ADVERTISE_HOST")
    if advertise:
        return f"{advertise}:{gateway_port}"
    base = _env("CLAW_GATEWAY_BASE_URL")
    if base:
        parsed = urlparse(base)
        if parsed.hostname and parsed.hostname not in (
            "127.0.0.1",
            "localhost",
            "host.docker.internal",
        ):
            return f"{parsed.hostname}:{parsed.port or gateway_port}"
    for key in ("CLAW_FC_WORKER_DATABASE_URL", "CLAW_GATEWAY_DATABASE_URL"):
        db = _env(key)
        if not db or "@" not in db:
            continue
        hostport = db.rsplit("@", 1)[-1].split("/", 1)[0]
        host = hostport.rsplit(":", 1)[0]
        if host and host not in ("127.0.0.1", "localhost", "postgres"):
            return f"{host}:{gateway_port}"
    return f"10.8.0.2:{gateway_port}"


def _machine_settings_for_fc(gateway_host: str, gateway_port: int) -> dict[str, Any]:
    path = ROOT / "deploy/stack/openvscode-settings.json"
    cfg = json.loads(path.read_text(encoding="utf-8"))
    cfg["claw.gatewayHost"] = gateway_host
    cfg["claw.gatewayPublicHost"] = _env("CLAW_GATEWAY_PUBLIC_HOST") or f"127.0.0.1:{gateway_port}"
    trusted = list(cfg.get("security.workspace.trust.trustedFolders") or [])
    for folder in (
        "/claw_ws",
        "/claw_ws/proj_1/home",
        "/claw_ws/proj_2/home",
        "/claw_ws/proj_3/home",
    ):
        if folder not in trusted:
            trusted.append(folder)
    cfg["security.workspace.trust.trustedFolders"] = trusted
    return cfg


def _install_restart_script(
    port: int,
    ext_ver: str,
    vsix_b64: str,
    machine_b64: str,
    gateway_host: str,
    gateway_public_host: str,
    proj_id: int,
) -> str:
    tmp_vsix = f"/tmp/claw-vscode-{ext_ver}.vsix"
    return f"""set -e
OVS_BIN="/home/.openvscode-server/bin/openvscode-server"
EXT_DIR=/opt/claw-extensions
SD=/opt/claw-ovs/server-data
OVS_HOME=/opt/claw-ovs/home
VSIX="{tmp_vsix}"
PORT={port}
EXT_VER={json.dumps(ext_ver)}

if [ ! -x "$OVS_BIN" ]; then
  echo "fc ovs install: openvscode-server missing (claw-ovs template)" >&2
  exit 127
fi

printf '%s' '{vsix_b64}' | base64 -d >"$VSIX"
if [ ! -s "$VSIX" ]; then
  echo "fc ovs install: decoded VSIX empty at $VSIX" >&2
  exit 1
fi

export HOME="$OVS_HOME"
mkdir -p "$OVS_HOME" "$EXT_DIR" "$SD/data/logs" {GUEST_CLAW_WS}

if "$OVS_BIN" --list-extensions --extensions-dir="$EXT_DIR" --server-data-dir="$SD" 2>/dev/null \\
  | grep -q '^claw\\.ovs-chat-demo$'; then
  "$OVS_BIN" --uninstall-extension claw.ovs-chat-demo \\
    --extensions-dir="$EXT_DIR" --server-data-dir="$SD" 2>/dev/null || true
fi

echo "==> install-extension $VSIX"
"$OVS_BIN" --install-extension "$VSIX" \\
  --extensions-dir="$EXT_DIR" \\
  --server-data-dir="$SD" \\
  --force

"$OVS_BIN" --list-extensions --extensions-dir="$EXT_DIR" --server-data-dir="$SD" 2>/dev/null \\
  | grep -q '^claw\\.claw-vscode$' || {{ echo "claw.claw-vscode not listed" >&2; exit 1; }}

/home/.openvscode-server/node --check "$EXT_DIR/claw.claw-vscode-$EXT_VER/extension.js"

MACHINE="$SD/Machine/settings.json"
MACHINE_DATA="$SD/data/Machine/settings.json"
mkdir -p "$(dirname "$MACHINE")" "$(dirname "$MACHINE_DATA")"
printf '%s' '{machine_b64}' | base64 -d >"$MACHINE"
cp -f "$MACHINE" "$MACHINE_DATA"
echo "==> Machine settings → $MACHINE and $MACHINE_DATA (claw.gatewayHost={gateway_host})"
curl -fsS -m 8 "http://{gateway_host}/healthz" >/dev/null \\
  || {{ echo "fc ovs install: gateway http://{gateway_host}/healthz not reachable from sandbox" >&2; exit 1; }}

WS_SETTINGS="{GUEST_CLAW_WS}/proj_{proj_id}/home/.vscode/settings.json"
mkdir -p "$(dirname "$WS_SETTINGS")"
python3 <<'PY'
import json, pathlib
p = pathlib.Path({json.dumps(f"{GUEST_CLAW_WS}/proj_{proj_id}/home/.vscode/settings.json")})
cfg: dict = {{}}
if p.is_file():
    try:
        cfg = json.loads(p.read_text(encoding="utf-8"))
    except json.JSONDecodeError:
        cfg = {{}}
cfg["claw.projId"] = {proj_id}
cfg["claw.gatewayHost"] = {json.dumps(gateway_host)}
cfg["claw.gatewayPublicHost"] = {json.dumps(gateway_public_host)}
p.write_text(json.dumps(cfg, indent=2, ensure_ascii=False) + "\\n", encoding="utf-8")
print(f"==> Workspace settings {{p}}")
PY

OVS_LOG="{GUEST_CLAW_WS}/.claw-ovs.log"
OVS_PID="{GUEST_CLAW_WS}/.claw-ovs.pid"
if [ -f "$OVS_PID" ]; then
  kill "$(cat "$OVS_PID")" 2>/dev/null || true
  rm -f "$OVS_PID"
fi
nohup "$OVS_BIN" \\
  --host=0.0.0.0 --port="$PORT" \\
  --without-connection-token \\
  --server-base-path=/ovs \\
  --extensions-dir="$EXT_DIR" \\
  --server-data-dir="$SD" \\
  --enable-proposed-api=claw.claw-vscode \\
  >"$OVS_LOG" 2>&1 &
echo $! >"$OVS_PID"
for _ in $(seq 1 45); do
  if curl -fsS --connect-timeout 2 "http://127.0.0.1:$PORT/ovs/" >/dev/null 2>&1; then
    echo "OK: openvscode /ovs/ ready (pid=$(cat "$OVS_PID"))"
    exit 0
  fi
  sleep 1
done
echo "fc ovs install: /ovs/ timeout (see $OVS_LOG)" >&2
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
    timeout: int = 300,
) -> str:
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
    try:
        parsed = json.loads(proc.stdout.decode("utf-8"))
    except json.JSONDecodeError as exc:
        raise RuntimeError(f"fc_exec bad json: {proc.stdout!r}") from exc
    if not parsed.get("ok"):
        raise RuntimeError(parsed.get("error") or "fc_exec failed")
    return str(parsed.get("stdout") or "")


def _service_public_host(port: int, sandbox_id: str, domain: str) -> str:
    return f"{port}-{sandbox_id}.{domain}"


def _ovs_base_url(sandbox_id: str, domain: str, ovs_port: int) -> str:
    host = _service_public_host(ovs_port, sandbox_id, domain)
    scheme = "http" if _is_self_hosted(_env("CLAW_FC_API_URL", "http://10.8.0.9:3000")) else "https"
    return f"{scheme}://{host}/ovs"


def _curl_ok(url: str) -> bool:
    proc = subprocess.run(
        ["curl", "-fsS", "--connect-timeout", "5", "--max-time", "20", "-o", "/dev/null", "-w", "%{http_code}", url],
        capture_output=True,
        text=True,
        check=False,
    )
    return proc.returncode == 0 and (proc.stdout or "").strip().startswith("2")


def main() -> int:
    parser = argparse.ArgumentParser(description="Runtime install claw-vscode in FC OVS singleton")
    parser.add_argument("--reuse", action="store_true", help="reuse existing ovs-singleton (default)")
    parser.add_argument("--proj-id", type=int, default=int(_env("CLAW_FC_E2E_PROJ_ID", "3") or "3"))
    parser.add_argument("--json", action="store_true")
    args = parser.parse_args()

    _load_dotenv(ROOT / ".env")

    api_key = _env("CLAW_FC_API_KEY") or _env("E2B_API_KEY") or _env("ALIYUN_E2B_TOKEN")
    if not api_key:
        print("error: set CLAW_FC_API_KEY in .env", file=sys.stderr)
        return 1

    api_url = _env("CLAW_FC_API_URL") or _env("E2B_API_URL") or "http://10.8.0.9:3000"
    sandbox_url = _env("CLAW_E2B_SANDBOX_URL") or _env("E2B_SANDBOX_URL")
    fc_domain = _env("CLAW_FC_DOMAIN") or _env("E2B_DOMAIN") or "supone.top"
    cluster_id = _env("CLAW_CLUSTER_ID") or "default"
    gateway_port = int(_env("GATEWAY_HOST_PORT", "8088") or "8088")
    ovs_port = _ovs_port()
    self_hosted = _is_self_hosted(api_url)
    ext_ver = _ext_version()
    gateway_host = _resolve_fc_ovs_gateway_host(gateway_port)
    gateway_public_host = _env("CLAW_GATEWAY_PUBLIC_HOST") or f"127.0.0.1:{gateway_port}"
    machine_b64 = base64.b64encode(
        json.dumps(_machine_settings_for_fc(gateway_host, gateway_port), ensure_ascii=False).encode("utf-8")
    ).decode("ascii")
    print(f"==> FC OVS claw.gatewayHost={gateway_host} (Remote EH → gateway agent/ws)", file=sys.stderr)

    vsix_host = _package_vsix_host()
    _stage_vsix_on_nas_optional(vsix_host)
    vsix_b64 = base64.b64encode(vsix_host.read_bytes()).decode("ascii")
    print(f"==> VSIX {vsix_host.name} ({vsix_host.stat().st_size} bytes) → sandbox base64 inject", file=sys.stderr)

    sandbox_id = _find_ovs_singleton(cluster_id, api_url, api_key, self_hosted)
    if not sandbox_id:
        print(f"==> no ovs-singleton for cluster={cluster_id}; run fc-ovs-up.sh first", file=sys.stderr)
        raise RuntimeError("missing ovs-singleton — run: ./deploy/stack/lib/fc-ovs-up.sh --reuse")

    print(f"==> ovs sandbox {sandbox_id}", file=sys.stderr)
    script = _install_restart_script(
        ovs_port, ext_ver, vsix_b64, machine_b64, gateway_host, gateway_public_host, args.proj_id
    )
    out = _run_fc_exec(
        sandbox_id=sandbox_id,
        script=script,
        api_key=api_key,
        api_url=api_url,
        sandbox_url=sandbox_url,
        domain=fc_domain,
        self_hosted=self_hosted,
    )
    if out.strip():
        print(out.strip(), file=sys.stderr)

    ovs_url = _ovs_base_url(sandbox_id, fc_domain, ovs_port)
    folder_url = f"{ovs_url.rstrip('/')}?folder={GUEST_CLAW_WS}/proj_{args.proj_id}/home"

    result = {
        "sandboxId": sandbox_id,
        "clusterId": cluster_id,
        "extVersion": ext_ver,
        "gatewayHost": gateway_host,
        "ovsUrl": ovs_url,
        "ovsFolderUrl": folder_url,
        "ovsRootOk": _curl_ok(f"{ovs_url.rstrip('/')}/") if ovs_url else False,
        "ovsFolderOk": _curl_ok(folder_url) if folder_url else False,
    }

    if args.json:
        print(json.dumps(result, indent=2, ensure_ascii=False))
    else:
        print()
        print("FC OVS claw-vscode runtime install")
        print(f"  sandboxId:     {sandbox_id}")
        print(f"  gatewayHost:   {gateway_host}")
        print(f"  extVersion:    {ext_ver}")
        print(f"  ovsFolderUrl:  {folder_url}")
        print(f"  traffic check: root={result['ovsRootOk']} folder={result['ovsFolderOk']}")
        print()
        print("Browser: playground /ovs?projId=N or ovsFolderUrl directly")

    ok = result["ovsRootOk"] and result["ovsFolderOk"]
    return 0 if ok else 2


if __name__ == "__main__":
    raise SystemExit(main())

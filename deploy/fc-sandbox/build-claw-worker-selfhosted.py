#!/usr/bin/env python3
"""Build claw-worker template on self-hosted e2bserver. Author: kejiqing"""
from __future__ import annotations

import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]


def _env(name: str, default: str = "") -> str:
    return os.environ.get(name, default).strip()


def _stage_binaries(staging: Path) -> None:
    tools = staging / "tools"
    tools.mkdir(parents=True, exist_ok=True)
    nas_tools = _env("CLAW_NAS_TOOLS_DIR")
    if nas_tools and (Path(nas_tools) / "claw").is_file():
        shutil.copy2(f"{nas_tools}/claw", staging / "claw")
        shutil.copy2(f"{nas_tools}/ttyd", staging / "ttyd")
        return
    worker = _env(
        "CLAW_FC_WORKER_IMAGE",
        "crpi-cf9vxpq3n8or17mw.cn-hangzhou.personal.cr.aliyuncs.com/passionke/claw-gateway-worker:release-v1.6.12",
    )
    rt = _env("CLAW_CONTAINER_RUNTIME", "podman")
    cid = subprocess.check_output([rt, "create", worker], text=True).strip()
    try:
        subprocess.check_call([rt, "cp", f"{cid}:/usr/local/bin/claw", str(staging / "claw")])
        subprocess.check_call([rt, "cp", f"{cid}:/usr/local/bin/ttyd", str(staging / "ttyd")])
    finally:
        subprocess.call([rt, "rm", "-f", cid], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


def main() -> int:
    api_key = _env("E2B_API_KEY", "e2b_53ae1fed82754c17ad8077fbc8bcdd90")
    api_url = _env("E2B_API_URL", "http://10.8.0.9:3000")
    sandbox_url = _env("E2B_SANDBOX_URL", "http://10.8.0.9:3002")
    domain = _env("E2B_DOMAIN", "10.8.0.9")
    alias = _env("CLAW_E2B_TEMPLATE", "claw-worker")

    os.environ.setdefault("E2B_API_KEY", api_key)
    os.environ.setdefault("E2B_API_URL", api_url)
    os.environ.setdefault("E2B_SANDBOX_URL", sandbox_url)
    os.environ.setdefault("E2B_DOMAIN", domain)

    dockerfile = (ROOT / "deploy/fc-sandbox/Dockerfile.claw-worker-selfhosted").read_text(
        encoding="utf-8"
    )

    with tempfile.TemporaryDirectory(prefix="claw-e2b-tpl-") as tmp:
        staging = Path(tmp)
        _stage_binaries(staging)
        from e2b import Template, default_build_logger

        template = Template().from_dockerfile(dockerfile, context=str(staging))
        Template.build(template, alias=alias, on_build_logs=default_build_logger())
    print(f"OK: template {alias!r} ready on {api_url}")
    return 0


if __name__ == "__main__":
    sys.exit(main())

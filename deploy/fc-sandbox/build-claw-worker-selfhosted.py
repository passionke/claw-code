#!/usr/bin/env python3
"""Build claw-worker template on self-hosted e2bserver. Author: kejiqing"""
from __future__ import annotations

import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from e2b_selfhosted_build import podman_platform_args


def _env(name: str, default: str = "") -> str:
    return os.environ.get(name, default).strip()


def _conn_opts() -> dict[str, str]:
    return {
        "api_key": _env("E2B_API_KEY", "e2b_53ae1fed82754c17ad8077fbc8bcdd90"),
        "api_url": _env("E2B_API_URL", "http://10.8.0.9:3000"),
        "domain": _env("E2B_DOMAIN", "10.8.0.9"),
    }


def _stage_claude_tap(staging: Path, rt: str) -> None:
    tap_image = _env(
        "CLAUDE_TAP_IMAGE",
        "crpi-cf9vxpq3n8or17mw.cn-hangzhou.personal.cr.aliyuncs.com/passionke/claw-tap:latest",
    )
    for candidate in (
        _env("CLAUDE_TAP_SOURCE_BIN"),
        shutil.which("claude-tap") or "",
        str(Path.home() / ".local/bin/claude-tap"),
    ):
        if candidate and Path(candidate).is_file():
            shutil.copy2(candidate, staging / "claude-tap")
            return
    try:
        subprocess.check_call(
            [rt, "pull", "--platform", "linux/amd64", tap_image],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        cid = subprocess.check_output(
            [rt, "create", "--platform", "linux/amd64", tap_image],
            text=True,
        ).strip()
        try:
            for path in ("/usr/local/bin/claude-tap", "/app/.venv/bin/claude-tap"):
                try:
                    subprocess.check_call(
                        [rt, "cp", f"{cid}:{path}", str(staging / "claude-tap")]
                    )
                    return
                except subprocess.CalledProcessError:
                    continue
        finally:
            subprocess.call([rt, "rm", "-f", cid], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    except subprocess.CalledProcessError:
        print("warn: claude-tap not staged (install uv tool install claw-tap or set CLAUDE_TAP_SOURCE_BIN)", file=sys.stderr)


def _stage_binaries(staging: Path) -> None:
    worker = _env(
        "CLAW_FC_WORKER_IMAGE",
        "crpi-cf9vxpq3n8or17mw.cn-hangzhou.personal.cr.aliyuncs.com/passionke/claw-gateway-worker:release-v1.6.13",
    )
    rt = _env("CLAW_CONTAINER_RUNTIME", "podman")
    if rt == "auto":
        rt = "podman"
    plat = podman_platform_args()
    cid = subprocess.check_output([rt, "create", *plat, worker], text=True).strip()
    try:
        subprocess.check_call([rt, "cp", f"{cid}:/usr/local/bin/claw", str(staging / "claw")])
        subprocess.check_call([rt, "cp", f"{cid}:/usr/local/bin/ttyd", str(staging / "ttyd")])
    finally:
        subprocess.call([rt, "rm", "-f", cid], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    _stage_claude_tap(staging, rt)


def _sudo_nfs_setup() -> str:
    return r""" && apt-get install -y --no-install-recommends sudo \
    && echo 'user ALL=(ALL) NOPASSWD: /bin/mount, /bin/umount, /usr/bin/mountpoint, /bin/mkdir, /bin/chown' > /etc/sudoers.d/claw-nfs \
    && chmod 440 /etc/sudoers.d/claw-nfs"""


def _dockerfile() -> str:
    sudo = _sudo_nfs_setup()
    return f"""FROM docker.1ms.run/library/debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends \\
    nfs-common ca-certificates curl python3 python3-pip{sudo} \\
    && pip3 install --no-cache-dir --break-system-packages claw-tap \\
      -i https://pypi.tuna.tsinghua.edu.cn/simple \\
    && rm -rf /var/lib/apt/lists/*
COPY claw /usr/local/bin/claw
COPY ttyd /usr/local/bin/ttyd
RUN chmod +x /usr/local/bin/claw /usr/local/bin/ttyd
"""


def main() -> int:
    opts = _conn_opts()
    alias = _env("CLAW_E2B_TEMPLATE", "claw-worker")
    verify = _env("CLAW_E2B_TEMPLATE_SKIP_VERIFY", "0") not in ("1", "true", "yes")

    os.environ.setdefault("E2B_API_KEY", opts["api_key"])
    os.environ.setdefault("E2B_API_URL", opts["api_url"])
    os.environ.setdefault("E2B_SANDBOX_URL", _env("E2B_SANDBOX_URL", "http://10.8.0.9:3002"))
    os.environ.setdefault("E2B_DOMAIN", opts["domain"])

    from e2b import Template, default_build_logger

    with tempfile.TemporaryDirectory(prefix="claw-e2b-tpl-") as tmp:
        staging = Path(tmp)
        worker_img = _env("CLAW_FC_WORKER_IMAGE") or _env("CLAW_PODMAN_IMAGE") or "worker image"
        print(f"==> staging claw/ttyd from {worker_img} …")
        _stage_binaries(staging)
        dockerfile_path = staging / "Dockerfile"
        dockerfile_path.write_text(_dockerfile(), encoding="utf-8")
        print(f"==> e2b template build (copy ctx) alias={alias!r}")
        template = Template(file_context_path=str(staging)).from_dockerfile(str(dockerfile_path))
        Template.build(template, alias=alias, on_build_logs=default_build_logger(), **opts)

    print(f"OK: template {alias!r} ready on {opts['api_url']}")
    if verify:
        return _verify(alias, opts)
    return 0


def _verify(alias: str, opts: dict[str, str]) -> int:
    from e2b import Sandbox

    print("==> verify: create sandbox + check ttyd/claw")
    sandbox = Sandbox.create(alias, timeout=900, **opts)
    try:
        print(f"sandbox_id: {sandbox.sandbox_id}")
        for cmd in ("command -v ttyd", "command -v claw"):
            r = sandbox.commands.run(cmd, timeout=120)
            print(f"$ {cmd} -> exit={r.exit_code} stdout={(r.stdout or '').strip()!r}")
            if r.exit_code not in (0, None):
                return r.exit_code or 1
    finally:
        sandbox.kill()
    return 0


if __name__ == "__main__":
    sys.exit(main())

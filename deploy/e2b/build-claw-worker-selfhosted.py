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

DEFAULT_WORKER_IMAGE = (
    "crpi-cf9vxpq3n8or17mw.cn-hangzhou.personal.cr.aliyuncs.com/"
    "passionke/claw-gateway-worker:release-v1.6.14"
)

WORKER_START_CMD = "/usr/local/bin/claw-worker-start"
WORKER_READY_CMD = "/usr/local/bin/claw-worker-ready"


def _env(name: str, default: str = "") -> str:
    return os.environ.get(name, default).strip()


def _conn_opts() -> dict[str, str]:
    return {
        "api_key": _env("E2B_API_KEY", "e2b_53ae1fed82754c17ad8077fbc8bcdd90"),
        "api_url": _env("E2B_API_URL", "http://10.8.0.1:3000"),
        "domain": _env("E2B_DOMAIN", "supone.top"),
    }


def _worker_base_image() -> str:
    return _env("CLAW_E2B_TEMPLATE_FROM_IMAGE") or _env("CLAW_E2B_WORKER_IMAGE", DEFAULT_WORKER_IMAGE)


def _container_runtime() -> str:
    rt = _env("CLAW_CONTAINER_RUNTIME", "podman")
    if rt == "auto":
        for candidate in ("podman", "docker"):
            if shutil.which(candidate):
                return candidate
        return "podman"
    return rt


def _stage_from_worker_tag(staging: Path, worker_image: str) -> None:
    """Pull CI worker tag (linux/amd64) and stage claw+ttyd into e2b build context."""
    rt = _container_runtime()
    platform = _env("CLAW_E2B_TEMPLATE_PLATFORM", "linux/amd64")
    print(f"==> stage binaries from worker tag {worker_image!r} via {rt} ({platform})")
    subprocess.check_call([rt, "pull", "--platform", platform, worker_image])
    cid = subprocess.check_output(
        [rt, "create", "--platform", platform, worker_image],
        text=True,
    ).strip()
    try:
        for name in ("claw", "ttyd"):
            dest = staging / name
            subprocess.check_call([rt, "cp", f"{cid}:/usr/local/bin/{name}", str(dest)])
            dest.chmod(0o755)
            probe = subprocess.check_output(["file", "-b", str(dest)], text=True).strip()
            print(f"  {name}: {probe}")
            if "x86-64" not in probe and "x86_64" not in probe:
                raise SystemExit(f"error: {name} is not amd64 ({probe})")
    finally:
        subprocess.call([rt, "rm", "-f", cid], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


def _worker_start_ready_install() -> str:
    return r"""RUN printf '%s\n' \
        '#!/bin/sh' \
        'set -eu' \
        'exec sleep infinity' \
        > /usr/local/bin/claw-worker-start \
    && printf '%s\n' \
        '#!/bin/sh' \
        'command -v claw >/dev/null 2>&1' \
        > /usr/local/bin/claw-worker-ready \
    && chmod +x /usr/local/bin/claw-worker-start /usr/local/bin/claw-worker-ready
"""


def _dockerfile_debian_copy() -> str:
    debian = _env(
        "CLAW_E2B_TEMPLATE_DEBIAN_IMAGE",
        "docker.1ms.run/library/debian:bookworm-slim",
    )
    return f"""FROM {debian}
RUN apt-get update && apt-get install -y --no-install-recommends \\
    nfs-common ca-certificates sudo \\
    && echo 'user ALL=(ALL) NOPASSWD: /bin/mount, /bin/umount, /usr/bin/mountpoint, /bin/mkdir, /bin/chown' > /etc/sudoers.d/claw-nfs \\
    && chmod 440 /etc/sudoers.d/claw-nfs \\
    && rm -rf /var/lib/apt/lists/*
COPY claw /usr/local/bin/claw
COPY ttyd /usr/local/bin/ttyd
RUN chmod +x /usr/local/bin/claw /usr/local/bin/ttyd
{_worker_start_ready_install()}"""


def _forbidden_http() -> bool:
    if _env("CLAW_E2B_TEMPLATE_BUILD_STRATEGY") == "http":
        return True
    return any(
        _env(key)
        for key in (
            "CLAW_E2B_TEMPLATE_HTTP_BASE",
            "CLAW_E2B_TEMPLATE_HTTP_BIND",
            "CLAW_E2B_TEMPLATE_HTTP_HOST",
            "CLAW_E2B_TEMPLATE_HTTP_PORT",
        )
    )


def main() -> int:
    opts = _conn_opts()
    alias = _env("CLAW_E2B_TEMPLATE", "claw-worker")
    strategy = _env("CLAW_E2B_TEMPLATE_BUILD_STRATEGY", "from_image")
    verify = _env("CLAW_E2B_TEMPLATE_SKIP_VERIFY", "0") not in ("1", "true", "yes")

    if _forbidden_http():
        print(
            "error: HTTP artifact template builds are forbidden. "
            "Use CLAW_E2B_TEMPLATE_BUILD_STRATEGY=from_image with a CI worker image tag.",
            file=sys.stderr,
        )
        return 2

    os.environ.setdefault("E2B_API_KEY", opts["api_key"])
    os.environ.setdefault("E2B_API_URL", opts["api_url"])
    os.environ.setdefault("E2B_SANDBOX_URL", _env("E2B_SANDBOX_URL", "http://10.8.0.1:3002"))
    os.environ.setdefault("E2B_DOMAIN", opts["domain"])

    from e2b import Template, default_build_logger

    with tempfile.TemporaryDirectory(prefix="claw-e2b-tpl-") as tmp:
        staging = Path(tmp)
        dockerfile_path = staging / "Dockerfile"

        if strategy == "from_image":
            worker_image = _worker_base_image()
            _stage_from_worker_tag(staging, worker_image)
            dockerfile_path.write_text(_dockerfile_debian_copy(), encoding="utf-8")
            print(f"==> e2b build worker_tag={worker_image!r} ctx={staging}")
            template = (
                Template(file_context_path=str(staging))
                .from_dockerfile(str(dockerfile_path))
                .set_start_cmd(WORKER_START_CMD, WORKER_READY_CMD)
            )
        elif strategy == "copy":
            copy_dir = _env("CLAW_E2B_TEMPLATE_COPY_DIR")
            if not copy_dir:
                print(
                    "error: CLAW_E2B_TEMPLATE_BUILD_STRATEGY=copy requires "
                    "CLAW_E2B_TEMPLATE_COPY_DIR with claw+ttyd.",
                    file=sys.stderr,
                )
                return 1
            src = Path(copy_dir)
            for name in ("claw", "ttyd"):
                if not (src / name).is_file():
                    print(f"error: missing {src / name}", file=sys.stderr)
                    return 1
            for name in ("claw", "ttyd"):
                (staging / name).write_bytes((src / name).read_bytes())
                (staging / name).chmod(0o755)
            dockerfile_path.write_text(_dockerfile_debian_copy(), encoding="utf-8")
            print(f"==> copy build ctx={staging} (from {copy_dir})")
            template = (
                Template(file_context_path=str(staging))
                .from_dockerfile(str(dockerfile_path))
                .set_start_cmd(WORKER_START_CMD, WORKER_READY_CMD)
            )
        else:
            print(f"unknown CLAW_E2B_TEMPLATE_BUILD_STRATEGY={strategy!r}", file=sys.stderr)
            return 1

        build = Template.build(template, alias=alias, on_build_logs=default_build_logger(), **opts)

    print(f"template_id: {build.template_id}")
    print(f"build_id: {build.build_id}")
    print(f"OK: template {alias!r} ({build.template_id}) ready on {opts['api_url']}")
    if verify:
        return _verify(build.template_id, opts)
    return 0


def _verify(template: str, opts: dict[str, str]) -> int:
    from e2b import Sandbox

    print(f"==> verify: create sandbox template={template!r} + check ttyd/claw")
    sandbox = Sandbox.create(template, timeout=900, **opts)
    try:
        print(f"sandbox_id: {sandbox.sandbox_id}")
        for cmd in (
            "command -v ttyd",
            "command -v claw",
            "test -x /usr/local/bin/claw-worker-start",
            "test -x /usr/local/bin/claw-worker-ready",
        ):
            r = sandbox.commands.run(cmd, timeout=120)
            out = (r.stdout or "").strip()
            print(f"$ {cmd} -> exit={r.exit_code} stdout={out!r}")
            if r.exit_code not in (0, None) and cmd.startswith("command -v"):
                return r.exit_code or 1
    finally:
        sandbox.kill()
    return 0


if __name__ == "__main__":
    sys.exit(main())

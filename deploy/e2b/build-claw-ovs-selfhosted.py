#!/usr/bin/env python3
"""Build claw-ovs template (debian base + HTTP bundle) on self-hosted e2b. Author: kejiqing"""
from __future__ import annotations

import json
import os
import shutil
import subprocess
import sys
import tarfile
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]


def _env(name: str, default: str = "") -> str:
    return os.environ.get(name, default).strip()


def _container_runtime() -> str:
    rt = _env("CLAW_CONTAINER_RUNTIME", "podman")
    return "podman" if rt == "auto" else rt


def _conn_opts() -> dict[str, str]:
    return {
        "api_key": _env("E2B_API_KEY", _env("CLAW_E2B_API_KEY", "e2b_53ae1fed82754c17ad8077fbc8bcdd90")),
        "api_url": _env("E2B_API_URL", _env("CLAW_E2B_API_URL", "http://10.8.0.1:3000")),
        "domain": _env("E2B_DOMAIN", _env("CLAW_E2B_DOMAIN", "supone.top")),
    }


def _stage_ovs_tree(staging: Path) -> None:
    ovs_image = _env(
        "CLAW_OVS_IMAGE",
        _env(
            "CLAW_OVS_UPSTREAM_IMAGE",
            "crpi-cf9vxpq3n8or17mw.cn-hangzhou.personal.cr.aliyuncs.com/passionke/openvscode-server:1.109.5-ovs-chat-amd64",
        ),
    )
    rt = _container_runtime()
    cid = subprocess.check_output([rt, "create", ovs_image], text=True).strip()
    try:
        for src, dst in (
            ("/home/.openvscode-server", staging / "openvscode-server"),
            ("/opt/claw-extensions", staging / "claw-extensions"),
            ("/opt/claw-ovs", staging / "claw-ovs"),
        ):
            subprocess.check_call([rt, "cp", f"{cid}:{src}", str(dst)])
        ext_ver = json.loads(
            (ROOT / "extensions/claw-vscode/package.json").read_text(encoding="utf-8")
        )["version"]
        vsix = ROOT / f"deploy/stack/claw.claw-vscode-{ext_ver}.vsix"
        if not vsix.is_file():
            subprocess.check_call(
                [str(ROOT / "deploy/stack/lib/package-claw-vscode-vsix.sh")],
                cwd=ROOT,
            )
        shutil.copy2(vsix, staging / "claw-ovs" / "claw-vscode.vsix")
    finally:
        subprocess.call([rt, "rm", "-f", cid], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


def _pack_bundle(staging: Path) -> Path:
    bundle = staging / "claw-ovs-bundle.tar.gz"
    with tarfile.open(bundle, "w:gz") as tar:
        for name in ("openvscode-server", "claw-extensions", "claw-ovs"):
            tar.add(staging / name, arcname=name)
    print(f"==> bundle {bundle} ({bundle.stat().st_size // (1024 * 1024)} MiB)")
    return bundle


def _sudo_nfs_setup() -> str:
    return r""" && apt-get install -y --no-install-recommends sudo \
    && echo 'user ALL=(ALL) NOPASSWD: /bin/mount, /bin/umount, /usr/bin/mountpoint, /bin/mkdir, /bin/chown' > /etc/sudoers.d/claw-nfs \
    && chmod 440 /etc/sudoers.d/claw-nfs"""


def _dockerfile_context(ovs_port: int) -> str:
    sudo = _sudo_nfs_setup()
    return f"""FROM docker.1ms.run/library/debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends \\
    nfs-common ca-certificates curl{sudo} \\
    && rm -rf /var/lib/apt/lists/* \\
    && mkdir -p /home /opt /claw_ws /tmp/ovs-bundle
COPY claw-ovs-bundle.tar.gz /tmp/claw-ovs-bundle.tar.gz
RUN tar -xz -C /tmp/ovs-bundle -f /tmp/claw-ovs-bundle.tar.gz \\
    && mv /tmp/ovs-bundle/openvscode-server /home/.openvscode-server \\
    && mv /tmp/ovs-bundle/claw-extensions /opt/claw-extensions \\
    && mv /tmp/ovs-bundle/claw-ovs /opt/claw-ovs \\
    && rm -rf /tmp/ovs-bundle /tmp/claw-ovs-bundle.tar.gz \\
    && chmod -R a+rwX /home/.openvscode-server /opt/claw-extensions /opt/claw-ovs /claw_ws
RUN printf '%s\\n' \\
        '#!/bin/sh' \\
        'set -eu' \\
        'export HOME=/opt/claw-ovs/home' \\
        'mkdir -p /opt/claw-ovs/home /opt/claw-extensions /opt/claw-ovs/server-data/data/logs /opt/claw-ovs/server-data/data/Machine /claw_ws' \\
        'exec /home/.openvscode-server/bin/openvscode-server --host=0.0.0.0 --port={ovs_port} --without-connection-token --server-base-path=/ovs --extensions-dir=/opt/claw-extensions --server-data-dir=/opt/claw-ovs/server-data --enable-proposed-api=claw.claw-vscode,claw.ovs-chat-demo' \\
        > /usr/local/bin/claw-ovs-start \\
    && printf '%s\\n' \\
        '#!/bin/sh' \\
        'exec curl -fsS --connect-timeout 2 http://127.0.0.1:{ovs_port}/ovs/' \\
        > /usr/local/bin/claw-ovs-ready \\
    && chmod +x /usr/local/bin/claw-ovs-start /usr/local/bin/claw-ovs-ready
"""


def _ovs_port() -> int:
    try:
        return int(_env("CLAW_E2B_OVS_PORT", "3000") or "3000")
    except ValueError:
        return 3000


def _ovs_start_cmd(port: int) -> str:
    _ = port
    return "/usr/local/bin/claw-ovs-start"


def _ovs_ready_cmd(port: int) -> str:
    _ = port
    return "/usr/local/bin/claw-ovs-ready"


def main() -> int:
    opts = _conn_opts()
    alias = _env("CLAW_E2B_OVS_TEMPLATE", "claw-ovs")
    ovs_port = _ovs_port()

    os.environ.setdefault("E2B_API_KEY", opts["api_key"])
    os.environ.setdefault("E2B_API_URL", opts["api_url"])
    os.environ.setdefault("E2B_SANDBOX_URL", _env("E2B_SANDBOX_URL", _env("CLAW_E2B_SANDBOX_URL", "http://10.8.0.1:3002")))
    os.environ.setdefault("E2B_DOMAIN", opts["domain"])

    from e2b import Template, default_build_logger

    with tempfile.TemporaryDirectory(prefix="claw-e2b-ovs-") as tmp:
        staging = Path(tmp)
        print("==> staging OVS tree from podman image …")
        _stage_ovs_tree(staging)
        _pack_bundle(staging)
        dockerfile_path = staging / "Dockerfile"
        dockerfile_path.write_text(_dockerfile_context(ovs_port), encoding="utf-8")
        print(f"==> build ctx={staging}")
        template = (
            Template(file_context_path=str(staging))
            .from_dockerfile(str(dockerfile_path))
            .set_start_cmd(_ovs_start_cmd(ovs_port), _ovs_ready_cmd(ovs_port))
        )
        print(f"==> template startCmd=openvscode-server :{ovs_port}/ovs")
        Template.build(template, alias=alias, on_build_logs=default_build_logger(), **opts)

    print(f"OK: template {alias!r} ready on {opts['api_url']}")
    return 0


if __name__ == "__main__":
    sys.exit(main())

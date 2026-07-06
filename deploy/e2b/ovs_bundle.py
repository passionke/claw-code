#!/usr/bin/env python3
"""Shared OpenVSCode bundle staging for claw-ovs and claw-worker-relaxed. Author: kejiqing"""
from __future__ import annotations

import json
import shutil
import subprocess
import tarfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
OVS_MACHINE_SETTINGS = ROOT / "deploy/stack/openvscode-settings.json"


def ovs_port() -> int:
    import os

    try:
        return int(os.environ.get("CLAW_E2B_OVS_PORT", "3000") or "3000")
    except ValueError:
        return 3000


def claw_vscode_ext_version() -> str:
    pkg = ROOT / "extensions/claw-vscode/package.json"
    return json.loads(pkg.read_text(encoding="utf-8"))["version"]


def ensure_claw_vscode_vsix(staging_claw_ovs: Path) -> str:
    ext_ver = claw_vscode_ext_version()
    vsix = ROOT / f"deploy/stack/claw.claw-vscode-{ext_ver}.vsix"
    if not vsix.is_file():
        subprocess.check_call(
            [str(ROOT / "deploy/stack/lib/package-claw-vscode-vsix.sh")],
            cwd=ROOT,
        )
    shutil.copy2(vsix, staging_claw_ovs / "claw-vscode.vsix")
    return ext_ver


def stage_ovs_tree(staging: Path, container_runtime: str, ovs_image: str) -> str:
    """Stage OVS tree + machine settings; returns claw-vscode extension version."""
    rt = container_runtime
    cid = subprocess.check_output([rt, "create", ovs_image], text=True).strip()
    try:
        for src, dst in (
            ("/home/.openvscode-server", staging / "openvscode-server"),
            ("/opt/claw-extensions", staging / "claw-extensions"),
            ("/opt/claw-ovs", staging / "claw-ovs"),
        ):
            subprocess.check_call([rt, "cp", f"{cid}:{src}", str(dst)])
        ext_ver = ensure_claw_vscode_vsix(staging / "claw-ovs")
    finally:
        subprocess.call([rt, "rm", "-f", cid], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    if not OVS_MACHINE_SETTINGS.is_file():
        raise FileNotFoundError(f"missing {OVS_MACHINE_SETTINGS}")
    shutil.copy2(OVS_MACHINE_SETTINGS, staging / "openvscode-settings.json")
    return ext_ver


def pack_ovs_bundle(staging: Path) -> Path:
    bundle = staging / "claw-ovs-bundle.tar.gz"
    with tarfile.open(bundle, "w:gz") as tar:
        for name in ("openvscode-server", "claw-extensions", "claw-ovs"):
            tar.add(staging / name, arcname=name)
    print(f"==> ovs bundle {bundle} ({bundle.stat().st_size // (1024 * 1024)} MiB)")
    return bundle


def _ovs_install_and_scripts_runfile(ovs_port_num: int, ext_ver: str, *, worker_relaxed: bool) -> str:
    """Install claw.claw-vscode + Machine settings (same as Containerfile.openvscode)."""
    port = ovs_port_num
    worker_start = ""
    if worker_relaxed:
        worker_start = rf""" \
    && printf '%s\n' \
        '#!/bin/sh' \
        'set -eu' \
        '/usr/local/bin/claw-ovs-start >/tmp/claw-ovs.log 2>&1 &' \
        'exec sleep infinity' \
        > /usr/local/bin/claw-worker-relaxed-start \
    && printf '%s\n' \
        '#!/bin/sh' \
        'set -eu' \
        'command -v claw >/dev/null 2>&1' \
        'test -x /usr/local/bin/claw-ovs-ready' \
        '/usr/local/bin/claw-ovs-ready' \
        > /usr/local/bin/claw-worker-relaxed-ready \
    && chmod +x /usr/local/bin/claw-worker-relaxed-start /usr/local/bin/claw-worker-relaxed-ready"""
    return rf"""RUN tar -xz -C /tmp/ovs-bundle -f /tmp/claw-ovs-bundle.tar.gz \
    && mv /tmp/ovs-bundle/openvscode-server /home/.openvscode-server \
    && mv /tmp/ovs-bundle/claw-extensions /opt/claw-extensions \
    && mv /tmp/ovs-bundle/claw-ovs /opt/claw-ovs \
    && rm -rf /tmp/ovs-bundle /tmp/claw-ovs-bundle.tar.gz \
    && mkdir -p /opt/claw-ovs/home /opt/claw-extensions \
        /opt/claw-ovs/server-data/data/logs /opt/claw-ovs/server-data/data/Machine \
        /opt/claw-ovs/server-data/Machine \
    && cp /tmp/openvscode-machine-settings.json /opt/claw-ovs/server-data/Machine/settings.json \
    && cp /opt/claw-ovs/server-data/Machine/settings.json /opt/claw-ovs/server-data/data/Machine/settings.json \
    && HOME=/opt/claw-ovs/home /home/.openvscode-server/bin/openvscode-server \
        --install-extension /opt/claw-ovs/claw-vscode.vsix \
        --extensions-dir=/opt/claw-extensions \
        --server-data-dir=/opt/claw-ovs/server-data \
        --force \
    && HOME=/opt/claw-ovs/home /home/.openvscode-server/bin/openvscode-server \
        --list-extensions --extensions-dir=/opt/claw-extensions --server-data-dir=/opt/claw-ovs/server-data \
        | grep -q '^claw\.claw-vscode$' \
    && HOME=/opt/claw-ovs/home /home/.openvscode-server/bin/openvscode-server \
        --uninstall-extension claw.ovs-chat-demo \
        --extensions-dir=/opt/claw-extensions --server-data-dir=/opt/claw-ovs/server-data 2>/dev/null || true \
    && chmod -R a+rwX /home/.openvscode-server /opt/claw-extensions /opt/claw-ovs \
    && printf '%s\n' \
        '#!/bin/sh' \
        'set -eu' \
        'export HOME=/opt/claw-ovs/home' \
        'mkdir -p /opt/claw-ovs/home /opt/claw-extensions /opt/claw-ovs/server-data/data/logs /opt/claw-ovs/server-data/data/Machine' \
        'exec /home/.openvscode-server/bin/openvscode-server --host=0.0.0.0 --port={port} --without-connection-token --server-base-path=/ovs --extensions-dir=/opt/claw-extensions --server-data-dir=/opt/claw-ovs/server-data --enable-proposed-api=claw.claw-vscode' \
        > /usr/local/bin/claw-ovs-start \
    && printf '%s\n' \
        '#!/bin/sh' \
        'exec curl -fsS --connect-timeout 2 http://127.0.0.1:{port}/ovs/' \
        > /usr/local/bin/claw-ovs-ready \
    && chmod +x /usr/local/bin/claw-ovs-start /usr/local/bin/claw-ovs-ready{worker_start}
"""


def relaxed_worker_ovs_install_runfile(ovs_port_num: int, ext_ver: str) -> str:
    return _ovs_install_and_scripts_runfile(ovs_port_num, ext_ver, worker_relaxed=True)


def ovs_singleton_install_runfile(ovs_port_num: int, ext_ver: str) -> str:
    return _ovs_install_and_scripts_runfile(ovs_port_num, ext_ver, worker_relaxed=False)

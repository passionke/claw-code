#!/usr/bin/env python3
"""Build claw-ovs template (debian base + HTTP bundle) on self-hosted e2b. Author: kejiqing"""
from __future__ import annotations

import os
import shutil
import subprocess
import sys
import tarfile
import tempfile
import threading
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]


def _env(name: str, default: str = "") -> str:
    return os.environ.get(name, default).strip()


def _container_runtime() -> str:
    rt = _env("CLAW_CONTAINER_RUNTIME", "podman")
    return "podman" if rt == "auto" else rt


def _conn_opts() -> dict[str, str]:
    return {
        "api_key": _env("E2B_API_KEY", _env("CLAW_FC_API_KEY", "e2b_53ae1fed82754c17ad8077fbc8bcdd90")),
        "api_url": _env("E2B_API_URL", _env("CLAW_FC_API_URL", "http://10.8.0.9:3000")),
        "domain": _env("E2B_DOMAIN", _env("CLAW_FC_DOMAIN", "10.8.0.9")),
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


def _dockerfile_http(base_url: str) -> str:
    b = base_url.rstrip("/")
    sudo = _sudo_nfs_setup()
    return f"""FROM docker.1ms.run/library/debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends \\
    nfs-common ca-certificates curl{sudo} \\
    && rm -rf /var/lib/apt/lists/* \\
    && mkdir -p /home /opt /claw_ws /tmp/ovs-bundle \\
    && curl -fsSL {b}/claw-ovs-bundle.tar.gz | tar -xz -C /tmp/ovs-bundle \\
    && mv /tmp/ovs-bundle/openvscode-server /home/.openvscode-server \\
    && mv /tmp/ovs-bundle/claw-extensions /opt/claw-extensions \\
    && mv /tmp/ovs-bundle/claw-ovs /opt/claw-ovs \\
    && rm -rf /tmp/ovs-bundle \\
    && chmod -R a+rwX /home/.openvscode-server /opt/claw-extensions /opt/claw-ovs /claw_ws
"""


def _make_handler(directory: Path) -> type[SimpleHTTPRequestHandler]:
    dir_str = str(directory)

    class Handler(SimpleHTTPRequestHandler):
        def __init__(self, *args: object, **kwargs: object) -> None:
            super().__init__(*args, directory=dir_str, **kwargs)

    return Handler


class _ArtifactServer:
    def __init__(self, directory: Path, host: str, port: int) -> None:
        self._directory = directory
        self._host = host
        self._port = port
        self._httpd: ThreadingHTTPServer | None = None
        self._thread: threading.Thread | None = None

    @property
    def base_url(self) -> str:
        return f"http://{self._host}:{self._port}"

    def __enter__(self) -> "_ArtifactServer":
        bind_host = _env("CLAW_E2B_TEMPLATE_HTTP_BIND", "0.0.0.0")
        handler = _make_handler(self._directory)
        self._httpd = ThreadingHTTPServer((bind_host, self._port), handler)
        self._thread = threading.Thread(target=self._httpd.serve_forever, daemon=True)
        self._thread.start()
        print(f"==> artifact HTTP {self.base_url} (dir={self._directory})")
        return self

    def __exit__(self, *_exc: object) -> None:
        if self._httpd:
            self._httpd.shutdown()
            self._httpd.server_close()
        if self._thread:
            self._thread.join(timeout=5)


def main() -> int:
    opts = _conn_opts()
    alias = _env("CLAW_FC_OVS_TEMPLATE", "claw-ovs")
    host = _env("CLAW_E2B_OVS_TEMPLATE_HTTP_HOST", _env("CLAW_E2B_TEMPLATE_HTTP_HOST", "10.8.0.2"))
    port = int(_env("CLAW_E2B_OVS_TEMPLATE_HTTP_PORT", "18889"))

    os.environ.setdefault("E2B_API_KEY", opts["api_key"])
    os.environ.setdefault("E2B_API_URL", opts["api_url"])
    os.environ.setdefault("E2B_SANDBOX_URL", _env("E2B_SANDBOX_URL", _env("CLAW_E2B_SANDBOX_URL", "http://10.8.0.9:3002")))
    os.environ.setdefault("E2B_DOMAIN", opts["domain"])

    from e2b import Template, default_build_logger

    with tempfile.TemporaryDirectory(prefix="claw-e2b-ovs-") as tmp:
        staging = Path(tmp)
        print("==> staging OVS tree from podman image …")
        _stage_ovs_tree(staging)
        _pack_bundle(staging)
        dockerfile = _dockerfile_http(f"http://{host}:{port}")
        with _ArtifactServer(staging, host, port) as server:
            print(f"==> http build artifacts={server.base_url!r}")
            template = Template().from_dockerfile(dockerfile)
            Template.build(template, alias=alias, on_build_logs=default_build_logger(), **opts)

    print(f"OK: template {alias!r} ready on {opts['api_url']}")
    return 0


if __name__ == "__main__":
    sys.exit(main())

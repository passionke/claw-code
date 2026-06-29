#!/usr/bin/env python3
"""Build claw-nas-api template (debian + python3, bound to NAS export root). Author: kejiqing

Bakes deploy/fc-sandbox/claw-nas-api/server.py into /opt/claw-nas-api. The gateway
launches it after sandbox create (see fc_nas_api_singleton), same pattern as observe.
"""
from __future__ import annotations

import os
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SERVER_SRC = ROOT / "deploy" / "fc-sandbox" / "claw-nas-api" / "server.py"


def _env(name: str, default: str = "") -> str:
    return os.environ.get(name, default).strip()


def _conn_opts() -> dict[str, str]:
    return {
        "api_key": _env("E2B_API_KEY", _env("CLAW_FC_API_KEY", "e2b_53ae1fed82754c17ad8077fbc8bcdd90")),
        "api_url": _env("E2B_API_URL", _env("CLAW_FC_API_URL", "http://10.8.0.9:3000")),
        "domain": _env("E2B_DOMAIN", _env("CLAW_FC_DOMAIN", "supone.top")),
    }


def _nfs_sudo() -> str:
    return (
        "apt-get install -y --no-install-recommends sudo "
        "&& echo 'user ALL=(ALL) NOPASSWD: /bin/mount, /bin/umount, /usr/bin/mountpoint, /bin/mkdir, /bin/chown' "
        "> /etc/sudoers.d/claw-nfs "
        "&& chmod 440 /etc/sudoers.d/claw-nfs"
    )


def _nas_api_port() -> int:
    try:
        return int(_env("CLAW_FC_NAS_API_PORT", "8090") or "8090")
    except ValueError:
        return 8090


def _install_nas_api_scripts(port: int) -> str:
    return (
        f"printf '%s\\n' "
        "'#!/bin/sh' "
        "'set -eu' "
        "'export CLAW_NAS_API_ROOT=/claw_ws' "
        "'export CLAW_NAS_API_LISTEN_HOST=0.0.0.0' "
        f"'export CLAW_NAS_API_LISTEN_PORT={port}' "
        "'exec python3 /opt/claw-nas-api/server.py' "
        "> /usr/local/bin/claw-nas-api-start "
        "&& printf '%s\\n' "
        "'#!/bin/sh' "
        f"'exec curl -fsS --connect-timeout 2 http://127.0.0.1:{port}/healthz' "
        "> /usr/local/bin/claw-nas-api-ready "
        "&& chmod +x /usr/local/bin/claw-nas-api-start /usr/local/bin/claw-nas-api-ready"
    )


def _nas_api_start_cmd(port: int) -> str:
    _ = port
    return "/usr/local/bin/claw-nas-api-start"


def _nas_api_ready_cmd(port: int) -> str:
    _ = port
    return "/usr/local/bin/claw-nas-api-ready"


def _build_template(staging: Path, port: int):
    from e2b import Template

    base_image = _env("CLAW_NAS_API_TEMPLATE_BASE_IMAGE", "debian:bookworm-slim")
    return (
        Template(file_context_path=str(staging))
        .from_image(base_image)
        .run_cmd(
            "apt-get update && apt-get install -y --no-install-recommends "
            "nfs-common ca-certificates curl python3 "
            f"&& {_nfs_sudo()} "
            "&& rm -rf /var/lib/apt/lists/* "
            "&& mkdir -p /opt/claw-nas-api /claw_ws",
            user="root",
        )
        .copy("server.py", "/opt/claw-nas-api/server.py", force_upload=True)
        .run_cmd(
            f"chmod -R a+rwX /opt/claw-nas-api /claw_ws && {_install_nas_api_scripts(port)}",
            user="root",
        )
        .set_start_cmd(_nas_api_start_cmd(port), _nas_api_ready_cmd(port))
    )


def _build_headers() -> dict[str, str]:
    headers: dict[str, str] = {}
    for env_key, header in (
        ("CLAW_FC_TEMPLATE_BUILD_MODE", "X-E2B-Template-Build-Mode"),
        ("CLAW_FC_TEMPLATE_SOURCE_REGISTRY_TYPE", "X-E2B-Template-Source-Registry-Type"),
        ("CLAW_FC_TEMPLATE_SOURCE_VPC_ID", "X-E2B-Template-Source-VPC-ID"),
        ("CLAW_FC_TEMPLATE_SOURCE_VSWITCH_IDS", "X-E2B-Template-Source-VSwitch-IDs"),
        ("CLAW_FC_TEMPLATE_SOURCE_SECURITY_GROUP_ID", "X-E2B-Template-Source-Security-Group-ID"),
        ("CLAW_FC_TEMPLATE_SOURCE_USERNAME", "X-E2B-Template-Source-Username"),
        ("CLAW_FC_TEMPLATE_SOURCE_PASSWORD", "X-E2B-Template-Source-Password"),
        ("CLAW_FC_TEMPLATE_DEST_IMAGE_REF", "X-E2B-Template-Dest-Image-Ref"),
        ("CLAW_FC_TEMPLATE_DEST_USERNAME", "X-E2B-Template-Dest-Username"),
        ("CLAW_FC_TEMPLATE_DEST_PASSWORD", "X-E2B-Template-Dest-Password"),
        (
            "CLAW_FC_TEMPLATE_SOURCE_ACREE_INSTANCE_ID",
            "X-E2B-Template-Source-ACREE-Instance-ID",
        ),
    ):
        val = _env(env_key)
        if val:
            headers[header] = val
    return headers


def main() -> int:
    if not SERVER_SRC.is_file():
        print(f"error: missing {SERVER_SRC}", file=sys.stderr)
        return 1
    opts = _conn_opts()
    alias = _env("CLAW_FC_NAS_API_TEMPLATE", "claw-nas-api")
    nas_port = _nas_api_port()

    os.environ.setdefault("E2B_API_KEY", opts["api_key"])
    os.environ.setdefault("E2B_API_URL", opts["api_url"])
    os.environ.setdefault(
        "E2B_SANDBOX_URL",
        _env("E2B_SANDBOX_URL", _env("CLAW_E2B_SANDBOX_URL", "http://10.8.0.9:3002")),
    )
    os.environ.setdefault("E2B_DOMAIN", opts["domain"])

    from e2b import Template, default_build_logger

    with tempfile.TemporaryDirectory(prefix="claw-e2b-nas-api-") as tmp:
        staging = Path(tmp)
        (staging / "server.py").write_bytes(SERVER_SRC.read_bytes())
        print(f"==> e2b Template.build alias={alias!r} (server.py baked into /opt/claw-nas-api)")
        template = _build_template(staging, nas_port)
        print(f"==> template startCmd=claw-nas-api :{nas_port}")
        headers = _build_headers()
        skip_cache = _env("CLAW_FC_TEMPLATE_SKIP_CACHE", "0") not in ("0", "false", "no")
        Template.build(
            template,
            name=alias,
            alias=alias,
            skip_cache=skip_cache,
            on_build_logs=default_build_logger(),
            headers=headers or None,
            **opts,
        )

    print(f"OK: template {alias!r} ready on {opts['api_url']}")
    return 0


if __name__ == "__main__":
    sys.exit(main())

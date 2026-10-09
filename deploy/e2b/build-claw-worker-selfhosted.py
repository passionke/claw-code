#!/usr/bin/env python3
"""Build claw-worker template on self-hosted e2bserver. Author: kejiqing"""
from __future__ import annotations

import os
import sys
import time
from pathlib import Path

_E2B_DIR = Path(__file__).resolve().parent
if str(_E2B_DIR) not in sys.path:
    sys.path.insert(0, str(_E2B_DIR))
from e2b_pg_settings import merge_settings_json_key
from e2b_template_content_hash import digest_parts, try_skip_unchanged
from e2b_template_registry import (
    apply_template_skip_cache_force,
    load_repo_dotenv,
    log_debian_base_resolution,
    template_gateway_worker_image,
)
from e2b_template_build import build_template_with_retry
from registry_extract import try_image_digest

ROOT = Path(__file__).resolve().parents[2]
load_repo_dotenv(ROOT)

WORKER_START_CMD = "/usr/local/bin/claw-worker-start"
WORKER_READY_CMD = "/usr/local/bin/claw-worker-ready"


def _env(name: str, default: str = "") -> str:
    return os.environ.get(name, default).strip()


def _conn_opts() -> dict[str, str]:
    return {
        "api_key": _env("E2B_API_KEY", _env("CLAW_E2B_API_KEY", "e2b_53ae1fed82754c17ad8077fbc8bcdd90")),
        "api_url": _env("E2B_API_URL", _env("CLAW_E2B_API_URL", "http://10.8.0.1:3000")),
        "domain": _env("E2B_DOMAIN", _env("CLAW_E2B_DOMAIN", "supone.top")),
    }


def _worker_base_image() -> str:
    return template_gateway_worker_image()


def _template_platform() -> str:
    return _env("CLAW_E2B_TEMPLATE_PLATFORM", "linux/amd64")


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
    log_debian_base_resolution(api_url=opts["api_url"])
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
    os.environ.setdefault(
        "E2B_SANDBOX_URL",
        _env("E2B_SANDBOX_URL", _env("CLAW_E2B_SANDBOX_URL", "http://10.8.0.1:3002")),
    )
    os.environ.setdefault("E2B_DOMAIN", opts["domain"])

    from e2b import Template, default_build_logger

    skip_cache = _env("CLAW_E2B_TEMPLATE_SKIP_CACHE", "0") not in ("0", "false", "no")
    content_digest = ""

    # The protocol image is real Debian and already contains claw/neuro; keep from_image.
    # Legacy claw-gateway-worker used to force debian+COPY; that path is retired.
    # Author: kejiqing
    worker_img = _worker_base_image()
    # e2bserver gates on image-name substrings; home canonical name includes debian-.
    # Author: kejiqing
    protocol_base = (
        (
            "debian-bookworm-claw-worker" in worker_img
            or "claw-worker-base" in worker_img
        )
        and "relaxed" not in worker_img
        and "claw-gateway-worker" not in worker_img
    )
    if strategy != "from_image" or not protocol_base:
        print(
            "error: e2b Worker templates require the independently published "
            "debian-bookworm-claw-worker protocol image",
            file=sys.stderr,
        )
        return 2

    worker_image = _worker_base_image()
    print(f"==> e2b Template.build from_image={worker_image!r} (protocol image)")
    content_digest = digest_parts(
        [
            ("image", worker_image.encode()),
            ("start", WORKER_START_CMD.encode()),
            ("ready", WORKER_READY_CMD.encode()),
        ]
    )
    if try_skip_unchanged("e2bWorker", content_digest, image_ref=worker_image):
        return 0
    template = Template().from_image(worker_image)
    template = template.set_start_cmd(WORKER_START_CMD, WORKER_READY_CMD)
    apply_template_skip_cache_force(template, skip_cache)
    build = build_template_with_retry(
        Template.build,
        label=alias,
        template=template,
        alias=alias,
        skip_cache=skip_cache,
        on_build_logs=default_build_logger(),
        **opts,
    )

    now_ms = int(time.time() * 1000)
    print(f"template_id: {build.template_id}")
    print(f"build_id: {build.build_id}")
    skip_pg = os.environ.get("CLAW_E2B_SKIP_WORKER_PG_PERSIST", "").strip().lower() in (
        "1",
        "true",
        "yes",
    )
    if skip_pg:
        print(
            f"==> skip PG e2bWorker.templateId (alias {alias!r} only; strict PG unchanged)",
            file=sys.stderr,
        )
    else:
        try:
            # Source image the worker template is derived from (CI/ACR worker tag).
            source_image = _worker_base_image()
            merge_settings_json_key(
                "e2bWorker",
                {
                    "templateId": build.template_id,
                    "buildId": build.build_id,
                    "contentHash": content_digest,
                    "alias": alias,
                    "imageRef": source_image,
                    "imageDigest": try_image_digest(source_image, platform=_template_platform()),
                    "updatedAtMs": now_ms,
                },
                now_ms=now_ms,
            )
            print(
                f"==> persisted e2bWorker.templateId={build.template_id!r} "
                f"buildId={build.build_id!r} to PG"
            )
        except Exception as exc:  # noqa: BLE001
            print(f"warn: skip PG e2bWorker.templateId persist: {exc}", file=sys.stderr)

    print(f"OK: template {alias!r} ({build.template_id}) ready on {opts['api_url']}")
    print(
        "hint: rebuild only updates PG; new build is used after gateway restart, "
        "manual worker reset, or when the sandbox is dead"
    )
    if verify:
        return _verify(build.template_id, opts)
    return 0


def _verify(template: str, opts: dict[str, str]) -> int:
    from e2b import Sandbox

    print(f"==> verify: create sandbox template={template!r} + check claw")
    sandbox = Sandbox.create(template, timeout=900, **opts)
    try:
        print(f"sandbox_id: {sandbox.sandbox_id}")
        for cmd in (
            "command -v claw",
            "command -v neuro-opencode",
            "command -v neuro-appserver",
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

#!/usr/bin/env python3
"""Build claw-worker template on self-hosted e2bserver. Author: kejiqing"""
from __future__ import annotations

import os
import shutil
import subprocess
import sys
from pathlib import Path

_E2B_DIR = Path(__file__).resolve().parent
if str(_E2B_DIR) not in sys.path:
    sys.path.insert(0, str(_E2B_DIR))
from e2b_template_registry import (
    apply_template_skip_cache_force,
    load_repo_dotenv,
    log_debian_base_resolution,
    template_debian_apt_mirror,
    template_gateway_worker_image,
)
from e2b_template_build import build_template_with_retry

ROOT = Path(__file__).resolve().parents[2]
load_repo_dotenv(ROOT)

DOCKERFILE_E2B = _E2B_DIR / "Dockerfile.claw-worker-selfhosted"
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


def _container_runtime() -> str:
    rt = _env("CLAW_CONTAINER_RUNTIME", "docker")
    if rt == "auto":
        for candidate in ("docker", "podman"):
            if shutil.which(candidate):
                return candidate
        return "docker"
    return rt


def _e2b_worker_image_tag(worker_image: str) -> str:
    """Canonical e2b from_image tag (home series). Author: kejiqing"""
    explicit = _env("CLAW_E2B_WORKER_E2B_IMAGE")
    if explicit:
        return explicit
    if ":" not in worker_image:
        return f"{worker_image}-debian"
    registry_repo, tag = worker_image.rsplit(":", 1)
    if "/" in registry_repo:
        registry, _repo = registry_repo.rsplit("/", 1)
        return f"{registry}/debian-bookworm-claw-worker:{tag}"
    return f"{registry_repo}/debian-bookworm-claw-worker:{tag}"


def _acr_registry_host(image_ref: str) -> str:
    if "/" not in image_ref:
        return ""
    return image_ref.split("/", 1)[0]


def _registry_login_if_needed(image_ref: str) -> None:
    registry = _acr_registry_host(image_ref)
    if not registry:
        return
    user = (
        _env("CLAW_REGISTRY_USER")
        or _env("ACR_USERNAME")
        or _env("ACR_USER")
        or _env("NEXUS_USER")
    )
    password = (
        _env("CLAW_REGISTRY_PASSWORD")
        or _env("ACR_PASSWORD")
        or _env("ACR_PASSWORK")
        or _env("NEXUS_PASSWORD")
    )
    if not user or not password:
        return
    rt = _container_runtime()
    if subprocess.call(
        [shutil.which(rt) or rt, "login", registry, "--get-login"],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    ) == 0:
        return
    print(f"==> {rt} login {registry!r}")
    subprocess.run(
        [rt, "login", registry, "-u", user, "--password-stdin"],
        input=password.encode(),
        check=True,
    )


def _build_e2b_worker_image(worker_image: str) -> str:
    """Layer e2b runtime on worker-series source; push debian-bookworm-* for Template.build."""
    # Host may prebuild/push (CLAW_E2B_PYTHON often has no docker). Author: kejiqing
    prebuilt = _env("CLAW_E2B_WORKER_E2B_IMAGE")
    if prebuilt and _env("CLAW_E2B_WORKER_SKIP_LOCAL_BUILD") in ("1", "true", "yes"):
        print(f"==> use prebuilt e2b worker image {prebuilt!r} (skip local docker build)")
        return prebuilt

    rt = _container_runtime()
    platform = _template_platform()
    e2b_image = _e2b_worker_image_tag(worker_image)
    if not DOCKERFILE_E2B.is_file():
        raise SystemExit(f"error: missing {DOCKERFILE_E2B}")

    apt_mirror = template_debian_apt_mirror()
    if apt_mirror:
        print(f"==> debian apt mirror (worker layer): {apt_mirror!r}")

    print(
        f"==> {rt} build e2b worker image {e2b_image!r} "
        f"(FROM {worker_image!r}, {platform}); e2b host will pull this image"
    )
    subprocess.check_call(
        [
            rt,
            "build",
            "-f",
            str(DOCKERFILE_E2B),
            "--build-arg",
            f"WORKER_BASE_IMAGE={worker_image}",
            "--build-arg",
            f"DEBIAN_APT_MIRROR={apt_mirror}",
            "--platform",
            platform,
            "-t",
            e2b_image,
            str(_E2B_DIR),
        ]
    )
    if _env("CLAW_E2B_WORKER_E2B_PUSH", "1") not in ("0", "false", "no"):
        _registry_login_if_needed(e2b_image)
        print(f"==> {rt} push {e2b_image!r}")
        subprocess.check_call([rt, "push", e2b_image])
    return e2b_image


def _is_worker_series_source(image: str) -> bool:
    """Worker-series protocol source only; e2b retag is produced by _build_e2b_worker_image."""
    if "claw-gateway-worker" in image or "relaxed" in image:
        return False
    return "claw-worker-base" in image


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
            "Use CLAW_E2B_TEMPLATE_BUILD_STRATEGY=from_image with a worker-series image.",
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
    worker_image = _worker_base_image()

    if strategy != "from_image" or not _is_worker_series_source(worker_image):
        print(
            "error: e2b Worker templates require from_image of the worker-series "
            "protocol image (…/claw-worker-base:<tag>)",
            file=sys.stderr,
        )
        return 2

    # Established path: source → debian-bookworm-claw-worker → Template.build.
    # Does not write Gateway PG; bind templateId in Admin. Author: kejiqing
    e2b_image = _build_e2b_worker_image(worker_image)
    print(f"==> e2b Template.build from_image={e2b_image!r}")
    template = Template().from_image(e2b_image)
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

    print(f"template_id: {build.template_id}")
    print(f"build_id: {build.build_id}")
    print(f"OK: template {alias!r} ({build.template_id}) ready on {opts['api_url']}")
    print(
        "hint: bind this templateId in Admin → e2b core components; "
        "Gateway uses it after save / worker reset"
    )
    if verify:
        return _verify(build.template_id, opts)
    return 0


def _verify(template: str, opts: dict[str, str]) -> int:
    from e2b import Sandbox

    print(f"==> verify: create sandbox template={template!r} + check protocol bins")
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

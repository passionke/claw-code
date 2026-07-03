#!/usr/bin/env python3
"""Build claw-observe: local podman build → push ACR → e2b from_image (same as claw-worker). Author: kejiqing"""
from __future__ import annotations

import os
import subprocess
import sys
import time
from pathlib import Path

_E2B_DIR = Path(__file__).resolve().parent
if str(_E2B_DIR) not in sys.path:
    sys.path.insert(0, str(_E2B_DIR))
from e2b_pg_settings import merge_settings_json_key
from e2b_template_registry import (
    apply_template_skip_cache_force,
    load_repo_dotenv,
    log_debian_base_resolution,
    template_debian_apt_mirror,
)

ROOT = Path(__file__).resolve().parents[2]
load_repo_dotenv(ROOT)

DOCKERFILE_E2B = _E2B_DIR / "Dockerfile.claw-observe-selfhosted"

DEFAULT_CLAUDE_TAP_IMAGE = "ghcr.io/passionke/claude-tap:v0.0.11"
DEFAULT_WORKER_IMAGE = (
    "crpi-cf9vxpq3n8or17mw.cn-hangzhou.personal.cr.aliyuncs.com/"
    "passionke/claw-gateway-worker:release-v1.6.17"
)

OBSERVE_START_CMD = "/usr/local/bin/claw-observe-start"
OBSERVE_READY_CMD = "/usr/local/bin/claw-observe-ready"


def _env(name: str, default: str = "") -> str:
    return os.environ.get(name, default).strip()


def _container_runtime() -> str:
    rt = _env("CLAW_CONTAINER_RUNTIME", "podman")
    return "podman" if rt == "auto" else rt


def _template_platform() -> str:
    return _env("CLAW_E2B_TEMPLATE_PLATFORM", "linux/amd64")


def _conn_opts() -> dict[str, str]:
    return {
        "api_key": _env("E2B_API_KEY", _env("CLAW_E2B_API_KEY", "e2b_53ae1fed82754c17ad8077fbc8bcdd90")),
        "api_url": _env("E2B_API_URL", _env("CLAW_E2B_API_URL", "http://10.8.0.1:3000")),
        "domain": _env("E2B_DOMAIN", _env("CLAW_E2B_DOMAIN", "supone.top")),
    }


def _acr_registry_host(image_ref: str) -> str:
    if "/" not in image_ref:
        return ""
    return image_ref.split("/", 1)[0]


def _acr_login_if_needed(image_ref: str) -> None:
    registry = _acr_registry_host(image_ref)
    if not registry:
        return
    user = _env("ACR_USERNAME") or _env("ACR_USER")
    password = _env("ACR_PASSWORD") or _env("ACR_PASSWORK")
    if not user or not password:
        return
    rt = _container_runtime()
    if subprocess.call(
        [rt, "login", registry, "--get-login"],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    ) == 0:
        return
    print(f"==> {rt} login {registry!r} (ACR_USERNAME)")
    subprocess.run(
        [rt, "login", registry, "-u", user, "--password-stdin"],
        input=password.encode(),
        check=True,
    )


def _tap_base_image() -> str:
    return _env("CLAUDE_TAP_IMAGE", DEFAULT_CLAUDE_TAP_IMAGE)


def _worker_release_tag() -> str:
    base = _env("CLAW_E2B_WORKER_IMAGE", DEFAULT_WORKER_IMAGE)
    if ":" not in base:
        return "latest"
    return base.rsplit(":", 1)[-1]


def _e2b_observe_image_tag(*, skip_cache: bool) -> str:
    """Observe ACR tag is separate from worker; rebuild must use a new tag (250 docker cache)."""
    explicit = _env("CLAW_E2B_OBSERVE_E2B_IMAGE")
    if explicit:
        return explicit

    base = _env("CLAW_E2B_WORKER_IMAGE", DEFAULT_WORKER_IMAGE)
    release_tag = _worker_release_tag()
    observe_tag = _env("CLAW_E2B_OBSERVE_E2B_TAG")
    if not observe_tag:
        observe_tag = f"{release_tag}-observe"
        if skip_cache:
            observe_tag = f"{observe_tag}.{time.strftime('%Y%m%d%H%M%S')}"
            print(
                f"==> new observe ACR tag {observe_tag!r} "
                "(set CLAW_E2B_OBSERVE_E2B_TAG to pin; content change needs new tag on 250)",
                file=sys.stderr,
            )

    if ":" not in base:
        return f"{base}/debian-bookworm-claw-observe:{observe_tag}"
    registry_repo, _ = base.rsplit(":", 1)
    if "/" in registry_repo:
        registry, _repo = registry_repo.rsplit("/", 1)
        return f"{registry}/debian-bookworm-claw-observe:{observe_tag}"
    return f"{registry_repo}/debian-bookworm-claw-observe:{observe_tag}"


def _build_e2b_observe_image(live_port: int, e2b_image: str) -> str:
    """Layer e2b observe on claude-tap locally; 250 pulls the pushed ACR image only."""
    rt = _container_runtime()
    platform = _template_platform()
    tap_base = _tap_base_image()
    apt_mirror = template_debian_apt_mirror()
    if not DOCKERFILE_E2B.is_file():
        raise SystemExit(f"error: missing {DOCKERFILE_E2B}")

    print(f"==> {rt} pull {tap_base!r} ({platform})")
    subprocess.check_call([rt, "pull", "--platform", platform, tap_base])

    if apt_mirror:
        print(
            f"==> debian apt mirror: {apt_mirror!r} "
            f"(CLAW_E2B_CN={_env('CLAW_E2B_CN') or '(unset)'}, "
            f"CLAW_E2B_DEBIAN_APT_MIRROR={_env('CLAW_E2B_DEBIAN_APT_MIRROR') or '(unset)'})"
        )

    print(
        f"==> {rt} build observe image {e2b_image!r} "
        f"(FROM {tap_base!r}, {platform}); 250 will pull this image"
    )
    build_args = [
        rt,
        "build",
        "-f",
        str(DOCKERFILE_E2B),
        "--build-arg",
        f"TAP_BASE_IMAGE={tap_base}",
        "--build-arg",
        f"OBSERVE_LIVE_PORT={live_port}",
        "--build-arg",
        f"DEBIAN_APT_MIRROR={apt_mirror}",
        "--platform",
        platform,
        "-t",
        e2b_image,
        str(_E2B_DIR),
    ]
    subprocess.check_call(build_args)
    if _env("CLAW_E2B_OBSERVE_E2B_PUSH", "1") not in ("0", "false", "no"):
        _acr_login_if_needed(e2b_image)
        print(f"==> {rt} push {e2b_image!r}")
        subprocess.check_call([rt, "push", e2b_image])
    return e2b_image


def _observe_live_port() -> int:
    try:
        return int(_env("CLAW_E2B_OBSERVE_LIVE_PORT", "3000") or "3000")
    except ValueError:
        return 3000


def main() -> int:
    opts = _conn_opts()
    log_debian_base_resolution(api_url=opts["api_url"])
    alias = _env("CLAW_E2B_OBSERVE_TEMPLATE", "claw-observe")
    live_port = _observe_live_port()

    os.environ.setdefault("E2B_API_KEY", opts["api_key"])
    os.environ.setdefault("E2B_API_URL", opts["api_url"])
    os.environ.setdefault(
        "E2B_SANDBOX_URL",
        _env("E2B_SANDBOX_URL", _env("CLAW_E2B_SANDBOX_URL", "http://10.8.0.1:3002")),
    )
    os.environ.setdefault("E2B_DOMAIN", opts["domain"])

    from e2b import Template, default_build_logger

    skip_cache = _env("CLAW_E2B_TEMPLATE_SKIP_CACHE", "0") not in ("0", "false", "no")

    e2b_image = _e2b_observe_image_tag(skip_cache=skip_cache)
    e2b_image = _build_e2b_observe_image(live_port, e2b_image)

    print(f"==> e2b Template.build from_image={e2b_image!r}")
    template = (
        Template()
        .from_image(e2b_image)
        .set_start_cmd(OBSERVE_START_CMD, OBSERVE_READY_CMD)
    )
    apply_template_skip_cache_force(template, skip_cache)
    build = Template.build(
        template,
        alias=alias,
        skip_cache=skip_cache,
        on_build_logs=default_build_logger(),
        **opts,
    )

    now_ms = int(time.time() * 1000)
    print(f"template_id: {build.template_id}")
    print(f"build_id: {build.build_id}")
    try:
        merge_settings_json_key(
            "e2bObserve",
            {
                "templateId": build.template_id,
                "alias": alias,
                "imageRef": e2b_image,
                "updatedAtMs": now_ms,
            },
            now_ms=now_ms,
        )
        print(f"==> persisted e2bObserve.templateId={build.template_id!r} to PG")
    except Exception as exc:  # noqa: BLE001 — build OK even if PG unreachable from Mac
        print(f"warn: skip PG e2bObserve.templateId persist: {exc}", file=sys.stderr)

    print(f"OK: template {alias!r} ({build.template_id}) ready on {opts['api_url']}")
    return 0


if __name__ == "__main__":
    sys.exit(main())

#!/usr/bin/env python3
"""e2b template base images + repo .env loader. Author: kejiqing"""
from __future__ import annotations

import os
import re
import sys
from pathlib import Path

_DEBIAN_DEFAULT = "debian:bookworm-slim"
_DEBIAN_CN = "docker.1ms.run/library/debian:bookworm-slim"
_CN_APT_MIRROR = "mirrors.aliyun.com"
_ACR_IMAGE_PREFIX = "crpi-cf9vxpq3n8or17mw.cn-hangzhou.personal.cr.aliyuncs.com/passionke"
_GHCR_IMAGE_PREFIX = "ghcr.io/passionke"
_DEFAULT_WORKER_RELEASE = "release-v1.6.17"
_DEFAULT_TAP_TAG_GLOBAL = "v0.0.11"
_DEFAULT_TAP_TAG_CN = "latest"


def _env(name: str) -> str:
    return os.environ.get(name, "").strip()


def _parse_region_value(raw: str) -> str:
    return raw.strip().strip('"').strip("'").lower()


def _region_from_bashrc() -> str:
    bashrc = Path.home() / ".bashrc"
    if not bashrc.is_file():
        return ""
    for line in bashrc.read_text(encoding="utf-8").splitlines():
        m = re.match(r"^\s*(?:export\s+)?region=(.+)$", line.strip())
        if m:
            return _parse_region_value(m.group(1))
    return ""


def _region_from_claw_region_file() -> str:
    path = Path.home() / ".claw-region"
    if not path.is_file():
        return ""
    for line in path.read_text(encoding="utf-8").splitlines():
        line = line.strip()
        if not line or line.startswith("#") or "=" not in line:
            continue
        key, _, val = line.partition("=")
        if key.strip().lower() in ("region", "region_name"):
            return _parse_region_value(val)
    return ""


def region_name() -> str:
    """Same keys as deploy/stack/lib/claw-region.sh (region / REGION / CLAW_REGION)."""
    for key in ("region", "REGION", "CLAW_REGION"):
        val = _env(key)
        if val:
            return _parse_region_value(val)
    from_file = _region_from_claw_region_file()
    if from_file:
        return from_file
    return _region_from_bashrc()


def region_is_china() -> bool:
    return region_name() == "china"


def cn_mirror_enabled() -> bool:
    """CN debian/apt mirrors; CI SG skips (matches claw-region.sh). Author: kejiqing"""
    if _env("GITHUB_ACTIONS").lower() == "true":
        return False
    return region_is_china()


def load_repo_dotenv(repo_root: Path | None = None) -> Path:
    """Load repo root `.env` into os.environ (does not override existing vars)."""
    root = repo_root or Path(__file__).resolve().parents[2]
    path = root / ".env"
    if not path.is_file():
        return root
    for raw in path.read_text(encoding="utf-8").splitlines():
        line = raw.strip()
        if not line or line.startswith("#") or "=" not in line:
            continue
        key, _, val = line.partition("=")
        key = key.strip()
        val = val.strip()
        if "#" in val:
            val = val.split("#", 1)[0].strip()
        val = val.strip('"').strip("'")
        if key and key not in os.environ:
            os.environ[key] = val
    return root


def template_image_prefix() -> str:
    """Registry namespace for passionke images; region=china → ACR (see claw-region.sh). Author: kejiqing"""
    if cn_mirror_enabled():
        return _ACR_IMAGE_PREFIX
    return _GHCR_IMAGE_PREFIX


def template_claude_tap_image() -> str:
    """claw-observe FROM; region=china → ACR claw-tap:latest. CLAUDE_TAP_IMAGE overrides."""
    explicit = _env("CLAUDE_TAP_IMAGE")
    if explicit:
        return explicit
    prefix = template_image_prefix()
    tag = _DEFAULT_TAP_TAG_CN if cn_mirror_enabled() else _DEFAULT_TAP_TAG_GLOBAL
    return f"{prefix}/claw-tap:{tag}"


def template_gateway_worker_image() -> str:
    """Worker template FROM; region=china → ACR. CLAW_E2B_* overrides."""
    explicit = _env("CLAW_E2B_TEMPLATE_FROM_IMAGE") or _env("CLAW_E2B_WORKER_IMAGE")
    if explicit:
        return explicit
    return f"{template_image_prefix()}/claw-gateway-worker:{_DEFAULT_WORKER_RELEASE}"


def template_debian_base_image() -> str:
    """Resolve debian bookworm-slim for e2b Template.build / Dockerfile FROM."""
    if cn_mirror_enabled() and _env("CLAW_USE_DOCKER_IO") != "1":
        return _DEBIAN_CN
    return _DEBIAN_DEFAULT


def template_debian_apt_mirror() -> str:
    """Debian apt mirror host; empty = keep image default sources."""
    if cn_mirror_enabled():
        return _CN_APT_MIRROR
    return ""


def template_apt_prepare_prefix() -> str:
    """Shell prepended before apt-get; e2bserver may inject tuna mirror that 403s on some hosts."""
    mirror = template_debian_apt_mirror()
    if not mirror:
        return ""
    return (
        f"sed -i 's|mirrors.tuna.tsinghua.edu.cn|{mirror}|g; "
        f"s|deb.debian.org|{mirror}|g; "
        f"s|security.debian.org|{mirror}|g' "
        "/etc/apt/sources.list.d/debian.sources 2>/dev/null || "
        f"sed -i 's|mirrors.tuna.tsinghua.edu.cn|{mirror}|g; "
        f"s|deb.debian.org|{mirror}|g; "
        f"s|security.debian.org|{mirror}|g' /etc/apt/sources.list 2>/dev/null || true; "
    )


def log_debian_base_resolution(*, api_url: str = "") -> str:
    """Print resolved debian ref; return it for callers."""
    img = template_debian_base_image()
    print(
        f"==> debian base: {img!r} (region={region_name() or '(unset)'})",
        file=sys.stderr,
    )
    mirror = template_debian_apt_mirror()
    if mirror:
        print(f"==> debian apt mirror: {mirror!r}", file=sys.stderr)
    if api_url:
        print(
            f"==> e2b build runs on server ({api_url}); "
            "docker pull logs also appear in e2bserver / template build output below",
            file=sys.stderr,
        )
    return img


def apply_template_skip_cache_force(template: object, skip_cache: bool) -> None:
    """Match e2b_selfhosted_build: skip_cache must set _force on template internals."""
    if not skip_cache:
        return
    inner = getattr(template, "_template", None)
    if inner is not None:
        inner._force = True


def e2b_python() -> str:
    """Prefer repo .venv-fc when present."""
    venv_py = Path(__file__).resolve().parents[2] / ".venv-fc" / "bin" / "python3"
    if venv_py.is_file():
        return str(venv_py)
    return sys.executable

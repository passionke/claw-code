#!/usr/bin/env python3
"""e2b template base images + repo .env loader. Author: kejiqing"""
from __future__ import annotations

import os
import sys
from pathlib import Path

_DEBIAN_DEFAULT = "debian:bookworm-slim"
_DEBIAN_CN = "docker.1ms.run/library/debian:bookworm-slim"


def _env(name: str) -> str:
    return os.environ.get(name, "").strip()


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


def template_debian_base_image() -> str:
    """Resolve debian bookworm-slim for e2b Template.build / Dockerfile FROM."""
    explicit = _env("CLAW_E2B_TEMPLATE_DEBIAN_IMAGE")
    if explicit:
        return explicit
    cn = _env("CLAW_E2B_CN").lower()
    if cn in ("1", "true", "yes", "on", "cn"):
        return _DEBIAN_CN
    return _DEBIAN_DEFAULT


def template_debian_apt_mirror() -> str:
    """Debian apt mirror host; empty = keep image default sources."""
    explicit = _env("CLAW_E2B_DEBIAN_APT_MIRROR")
    if explicit:
        return explicit
    if _env("CLAW_E2B_CN").lower() in ("1", "true", "yes", "on", "cn"):
        return "mirrors.aliyun.com"
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
        f"==> debian base: {img!r} "
        f"(CLAW_E2B_CN={_env('CLAW_E2B_CN') or '(unset)'}, "
        f"CLAW_E2B_TEMPLATE_DEBIAN_IMAGE={_env('CLAW_E2B_TEMPLATE_DEBIAN_IMAGE') or '(unset)'})",
        file=sys.stderr,
    )
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

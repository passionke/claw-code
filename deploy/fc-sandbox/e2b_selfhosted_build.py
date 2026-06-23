#!/usr/bin/env python3
"""Self-hosted e2b template build helpers. Author: kejiqing"""
from __future__ import annotations

import os


def _env(name: str, default: str = "") -> str:
    return os.environ.get(name, default).strip()


def podman_platform_args() -> list[str]:
    """FC sandboxes are linux/amd64; Mac arm64 host must extract with matching platform."""
    plat = _env("CLAW_E2B_TEMPLATE_PLATFORM", "linux/amd64")
    return ["--platform", plat] if plat else []

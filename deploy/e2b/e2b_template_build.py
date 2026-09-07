#!/usr/bin/env python3
"""Retry wrapper for e2b Template.build (GHA push flakes e.g. GHCR unknown blob). Author: kejiqing"""
from __future__ import annotations

import os
import time
from typing import Any, Callable


def _env(name: str, default: str = "") -> str:
    return os.environ.get(name, default).strip()


def template_build_retries() -> int:
    raw = _env("CLAW_E2B_TEMPLATE_BUILD_RETRIES", "3")
    try:
        n = int(raw)
    except ValueError:
        n = 3
    return max(1, min(n, 8))


def is_transient_template_build_error(exc: BaseException) -> bool:
    msg = str(exc).lower()
    needles = (
        "ci build failed",
        "unknown blob",
        "runtime error",
        "timeout",
        "temporarily unavailable",
        "connection reset",
        "503",
        "502",
        "504",
    )
    return any(n in msg for n in needles)


def build_template_with_retry(
    build_fn: Callable[..., Any],
    *,
    label: str,
    **kwargs: Any,
) -> Any:
    """Call Template.build via build_fn(**kwargs) with retries on transient CI errors."""
    attempts = template_build_retries()
    last: BaseException | None = None
    for i in range(1, attempts + 1):
        try:
            if i > 1:
                # Force rebuild path on retry — cached context can re-hit bad GHCR layers.
                kwargs["skip_cache"] = True
                print(
                    f"==> retry Template.build {label!r} attempt={i}/{attempts} (skip_cache=True)",
                    flush=True,
                )
            return build_fn(**kwargs)
        except BaseException as exc:  # noqa: BLE001 — e2b raises BuildException
            last = exc
            if i >= attempts or not is_transient_template_build_error(exc):
                raise
            wait_s = min(15 * i, 60)
            print(
                f"warn: Template.build {label!r} failed ({exc}); sleep {wait_s}s then retry",
                flush=True,
            )
            time.sleep(wait_s)
    assert last is not None
    raise last

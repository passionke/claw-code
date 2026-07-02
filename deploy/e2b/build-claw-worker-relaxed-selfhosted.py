#!/usr/bin/env python3
"""Build claw-worker-relaxed template (root exec profile) on self-hosted e2b. Author: kejiqing

Thin wrapper over build-claw-worker-selfhosted.py: same image, distinct alias.
strict/relaxed exec-user enforcement stays at the gateway exec layer (KISS:
one Dockerfile, two aliases selected by worker_profile_json.mode).
"""
from __future__ import annotations

import os
import runpy
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent


def main() -> int:
    os.environ.setdefault(
        "CLAW_E2B_TEMPLATE",
        os.environ.get("CLAW_E2B_TEMPLATE_RELAXED", "claw-worker-relaxed"),
    )
    os.environ.setdefault("CLAW_WORKER_EXEC_PROFILE", "relaxed")
    sys.argv = [str(HERE / "build-claw-worker-selfhosted.py")]
    runpy.run_path(str(HERE / "build-claw-worker-selfhosted.py"), run_name="__main__")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

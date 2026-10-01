#!/usr/bin/env python3
"""Build claw-worker-opencode (neuro-harness) on self-hosted e2b; see e2b_engine_worker.py. Author: kejiqing"""
from __future__ import annotations

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from e2b_engine_worker import build  # noqa: E402

if __name__ == "__main__":
    raise SystemExit(build("opencode"))

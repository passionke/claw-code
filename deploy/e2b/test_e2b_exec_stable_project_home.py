#!/usr/bin/env python3
# Contract test: solve exec exports stable NAS project config root. Author: kejiqing
"""Smoke tests for deploy/e2b/e2b_exec.py project config root."""

from __future__ import annotations

import unittest
from pathlib import Path

FC_EXEC = Path(__file__).resolve().parent / "e2b_exec.py"
STABLE_ROOT = "CLAW_PROJECT_CONFIG_ROOT=/claw_ds/project_home_def"


class FcExecStableProjectHomeTests(unittest.TestCase):
    def test_exec_solve_exports_stable_project_home_def(self) -> None:
        text = FC_EXEC.read_text(encoding="utf-8")
        self.assertIn(STABLE_ROOT, text)
        self.assertNotIn('CLAW_PROJECT_CONFIG_ROOT=/claw_ds"\n', text)


if __name__ == "__main__":
    unittest.main()

#!/usr/bin/env python3
# Contract test: solve exec exports stable NAS project config root. Author: kejiqing
"""Smoke tests for deploy/e2b/e2b_exec.py project config root and run_sh env."""

from __future__ import annotations

import importlib.util
import unittest
from pathlib import Path

FC_EXEC = Path(__file__).resolve().parent / "e2b_exec.py"
STABLE_ROOT = "CLAW_PROJECT_CONFIG_ROOT=/claw_ds/project_home_def"


def _load_e2b_exec():
    spec = importlib.util.spec_from_file_location("e2b_exec", FC_EXEC)
    mod = importlib.util.module_from_spec(spec)
    assert spec.loader is not None
    spec.loader.exec_module(mod)
    return mod


class FcExecStableProjectHomeTests(unittest.TestCase):
    def test_exec_solve_exports_stable_project_home_def(self) -> None:
        text = FC_EXEC.read_text(encoding="utf-8")
        self.assertIn(STABLE_ROOT, text)
        self.assertNotIn('CLAW_PROJECT_CONFIG_ROOT=/claw_ds"\n', text)


class FcExecRunShEnvTests(unittest.TestCase):
    def test_env_exports_sh_skips_blank_values(self) -> None:
        e2b_exec = _load_e2b_exec()
        out = e2b_exec._env_exports_sh(
            {
                "OPENAI_API_KEY": "claw-tap-cluster",
                "OPENAI_BASE_URL": "http://8080-sbx.supone.top",
                "EMPTY": "",
            }
        )
        self.assertIn('export OPENAI_API_KEY="claw-tap-cluster"', out)
        self.assertIn('export OPENAI_BASE_URL="http://8080-sbx.supone.top"', out)
        self.assertNotIn("EMPTY", out)

    def test_prepend_env_exports_wraps_run_sh_script(self) -> None:
        e2b_exec = _load_e2b_exec()
        script = e2b_exec._prepend_env_exports(
            "echo ok",
            {"CLAW_DEFAULT_MODEL": "openai/mimo-v2.5"},
        )
        self.assertIn("set -eu", script)
        self.assertIn('export CLAW_DEFAULT_MODEL="openai/mimo-v2.5"', script)
        self.assertIn("echo ok", script)


if __name__ == "__main__":
    unittest.main()

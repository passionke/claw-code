#!/usr/bin/env python3
# Tests for observe singleton discovery in e2b-tap-live-up. Author: kejiqing
"""Unit tests for deploy/e2b/e2b-tap-live-up.py."""

from __future__ import annotations

import importlib.util
import sys
import unittest
from pathlib import Path

_MODULE_PATH = Path(__file__).resolve().parent / "e2b-tap-live-up.py"
_spec = importlib.util.spec_from_file_location("e2b_tap_live_up", _MODULE_PATH)
assert _spec and _spec.loader
mod = importlib.util.module_from_spec(_spec)
sys.modules["e2b_tap_live_up"] = mod
_spec.loader.exec_module(mod)


class ObserveSingletonDiscoveryTests(unittest.TestCase):
    def test_running_state_required(self) -> None:
        self.assertTrue(mod._is_running_sandbox({"state": "running"}))
        self.assertFalse(mod._is_running_sandbox({"state": "killed"}))
        self.assertFalse(mod._is_running_sandbox({"state": "paused"}))

    def test_find_skips_killed_observe(self) -> None:
        rows = [
            {
                "sandboxID": "sbx_dead",
                "state": "killed",
                "metadata": {"clawRole": "observe-singleton", "clusterId": "local-dev"},
            },
            {
                "sandboxID": "sbx_live",
                "state": "running",
                "metadata": {"clawRole": "observe-singleton", "clusterId": "local-dev"},
            },
        ]

        def fake_list(_api: str, _key: str, _self_hosted: bool) -> list[dict]:
            return rows

        orig = mod._list_sandboxes
        mod._list_sandboxes = fake_list  # type: ignore[assignment]
        try:
            sid = mod._find_observe_singleton("local-dev", "http://x", "k", True)
        finally:
            mod._list_sandboxes = orig  # type: ignore[assignment]
        self.assertEqual(sid, "sbx_live")


if __name__ == "__main__":
    unittest.main()

#!/usr/bin/env python3
# Tests for claw-nas-api symlink atomic replace. Author: kejiqing
"""Unit tests for deploy/e2b/claw-nas-api/server.py."""

from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

from server import _atomic_symlink


class AtomicSymlinkTests(unittest.TestCase):
    def test_creates_symlink_to_version_dir(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            home = root / "home"
            home.mkdir()
            target = home / ".claw" / "project-home-versions" / "rev-a"
            target.mkdir(parents=True)
            (target / "CLAUDE.md").write_text("# v1\n", encoding="utf-8")
            link = home / "project_home_def"
            _atomic_symlink(link, ".claw/project-home-versions/rev-a")
            self.assertTrue(link.is_symlink())
            self.assertEqual(link.resolve(), target.resolve())
            self.assertEqual((link / "CLAUDE.md").read_text(encoding="utf-8"), "# v1\n")

    def test_replaces_existing_symlink_without_window(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            home = root / "home"
            home.mkdir()
            old = home / ".claw" / "project-home-versions" / "rev-old"
            new = home / ".claw" / "project-home-versions" / "rev-new"
            old.mkdir(parents=True)
            new.mkdir(parents=True)
            (old / "marker").write_text("old", encoding="utf-8")
            (new / "marker").write_text("new", encoding="utf-8")
            link = home / "project_home_def"
            _atomic_symlink(link, ".claw/project-home-versions/rev-old")
            self.assertEqual((link / "marker").read_text(encoding="utf-8"), "old")
            _atomic_symlink(link, ".claw/project-home-versions/rev-new")
            self.assertTrue(link.is_symlink())
            self.assertEqual((link / "marker").read_text(encoding="utf-8"), "new")

    def test_refuses_to_replace_real_directory(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            link = root / "project_home_def"
            link.mkdir()
            with self.assertRaises(ValueError) as ctx:
                _atomic_symlink(link, ".claw/project-home-versions/rev-a")
            self.assertIn("refusing to replace directory", str(ctx.exception))


if __name__ == "__main__":
    unittest.main()

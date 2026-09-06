#!/usr/bin/env python3
# Tests for claw-nas-api symlink atomic replace. Author: kejiqing
"""Unit tests for deploy/e2b/claw-nas-api/server.py."""

from __future__ import annotations

import gzip
import io
import tarfile
import tempfile
import unittest
from pathlib import Path

from server import (
    _atomic_symlink,
    _extract_git_import_tar,
    _finish_git_import_parts,
    _git_import_upload_staging,
)


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


def _tar_gz_bytes(files: dict[str, bytes]) -> bytes:
  buf = io.BytesIO()
  with tarfile.open(fileobj=buf, mode="w:") as archive:
    for rel, data in files.items():
      info = tarfile.TarInfo(name=rel)
      info.size = len(data)
      archive.addfile(info, io.BytesIO(data))
  raw = buf.getvalue()
  out = io.BytesIO()
  with gzip.GzipFile(fileobj=out, mode="wb") as gz:
    gz.write(raw)
  return out.getvalue()


class ExtractGitImportTarTests(unittest.TestCase):
    def test_extract_replaces_dest_tree(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            dest = root / "cluster" / "proj_1" / "home" / "repo"
            dest.mkdir(parents=True)
            (dest / "old.txt").write_text("old", encoding="utf-8")
            tar_gz = _tar_gz_bytes(
                {
                    "README.md": b"# new\n",
                    "src/a.rs": b"fn main() {}\n",
                }
            )
            count = _extract_git_import_tar(dest, tar_gz)
            self.assertEqual(count, 2)
            self.assertFalse((dest / "old.txt").exists())
            self.assertEqual((dest / "README.md").read_bytes(), b"# new\n")
            self.assertEqual((dest / "src/a.rs").read_bytes(), b"fn main() {}\n")

    def test_extract_rejects_path_traversal(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            dest = Path(tmp) / "cluster" / "proj_1" / "home" / "repo"
            tar_gz = _tar_gz_bytes({"../escape.txt": b"x"})
            with self.assertRaises(ValueError):
                _extract_git_import_tar(dest, tar_gz)


class FinishGitImportPartsTests(unittest.TestCase):
    def test_finish_concat_parts_then_extract_once(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            import server

            prev = server.NAS_ROOT
            server.NAS_ROOT = root
            try:
                dest_rel = "cluster/proj_9/home/starfish"
                tar_gz = _tar_gz_bytes(
                    {
                        "README.md": b"# chunked\n",
                        "pkg/main.py": b"print('ok')\n",
                    }
                )
                mid = len(tar_gz) // 2 or 1
                chunks = [tar_gz[:mid], tar_gz[mid:]]
                upload_id = "test-upload-1"
                staging = _git_import_upload_staging(upload_id)
                staging.mkdir(parents=True)
                for i, chunk in enumerate(chunks):
                    (staging / f"part-{i:04d}").write_bytes(chunk)
                result = _finish_git_import_parts(dest_rel, upload_id, len(chunks))
                self.assertEqual(result["parts"], len(chunks))
                self.assertEqual(result["extracted"], 2)
                dest = root / dest_rel
                self.assertEqual((dest / "README.md").read_bytes(), b"# chunked\n")
                self.assertEqual((dest / "pkg/main.py").read_bytes(), b"print('ok')\n")
                self.assertFalse(staging.exists())
            finally:
                server.NAS_ROOT = prev


if __name__ == "__main__":
    unittest.main()

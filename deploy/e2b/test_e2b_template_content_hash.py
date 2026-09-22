"""Unit tests for template content fingerprints. Author: kejiqing"""
from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

from e2b_template_content_hash import digest_parts, digest_tree, should_skip_publish


class ContentHashTests(unittest.TestCase):
    def test_digest_is_order_independent(self) -> None:
        a = digest_parts([("claw", b"bin"), ("dockerfile", b"FROM debian")])
        b = digest_parts([("dockerfile", b"FROM debian"), ("claw", b"bin")])
        self.assertEqual(a, b)

    def test_digest_changes_when_bytes_change(self) -> None:
        a = digest_parts([("claw", b"v1")])
        b = digest_parts([("claw", b"v2")])
        self.assertNotEqual(a, b)

    def test_skip_requires_matching_hash_and_build_id(self) -> None:
        digest = digest_parts([("claw", b"same")])
        self.assertTrue(should_skip_publish(digest, "build-1", digest))
        self.assertFalse(should_skip_publish("", "build-1", digest))
        self.assertFalse(should_skip_publish(digest, "", digest))
        self.assertFalse(should_skip_publish(digest, "build-1", digest + "x"))

    def test_tree_includes_nested_files(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "claw.bin").write_bytes(b"elf")
            nested = root / "ovs"
            nested.mkdir()
            (nested / "bundle").write_bytes(b"ovs")
            first = digest_tree(root, [("base", b"debian")])
            (nested / "bundle").write_bytes(b"ovs2")
            second = digest_tree(root, [("base", b"debian")])
        self.assertNotEqual(first, second)


if __name__ == "__main__":
    unittest.main()

#!/usr/bin/env python3
# Author: kejiqing
"""registry_scheme: HTTPS for 443; HTTP for private Nexus ports like :8082."""
from __future__ import annotations

import unittest

from registry_extract import _SCHEME_CACHE, _registry_url, registry_scheme


class RegistrySchemeTest(unittest.TestCase):
    def setUp(self) -> None:
        _SCHEME_CACHE.clear()

    def test_https_default_no_port(self) -> None:
        self.assertEqual(registry_scheme("ghcr.io"), "https")
        self.assertEqual(
            _registry_url("ghcr.io", "/v2/foo/manifests/latest"),
            "https://ghcr.io/v2/foo/manifests/latest",
        )

    def test_http_for_nexus_pull_port(self) -> None:
        self.assertEqual(registry_scheme("repo.550w.com:8082"), "http")
        self.assertEqual(
            _registry_url("repo.550w.com:8082", "/v2/passionke/claw-code/manifests/latest"),
            "http://repo.550w.com:8082/v2/passionke/claw-code/manifests/latest",
        )

    def test_https_for_443(self) -> None:
        self.assertEqual(registry_scheme("crpi.example.com:443"), "https")


if __name__ == "__main__":
    unittest.main()

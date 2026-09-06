#!/usr/bin/env python3
"""Tests for e2b_template_registry region switch. Author: kejiqing"""
from __future__ import annotations

import os
import unittest
from unittest.mock import patch

from e2b_template_registry import (
    cn_mirror_enabled,
    region_is_china,
    region_name,
    template_claude_tap_image,
    template_debian_apt_mirror,
    template_debian_base_image,
    template_gateway_worker_image,
)


class RegionSwitchTests(unittest.TestCase):
    def tearDown(self) -> None:
        for key in (
            "region",
            "REGION",
            "CLAW_REGION",
            "GITHUB_ACTIONS",
            "CLAW_USE_DOCKER_IO",
        ):
            os.environ.pop(key, None)

    def test_china_from_region_env(self) -> None:
        os.environ["region"] = "china"
        self.assertTrue(region_is_china())
        self.assertTrue(cn_mirror_enabled())
        self.assertEqual(template_debian_base_image(), "docker.1ms.run/library/debian:bookworm-slim")
        self.assertEqual(template_debian_apt_mirror(), "mirrors.aliyun.com")
        self.assertEqual(
            template_claude_tap_image(),
            "crpi-cf9vxpq3n8or17mw.cn-hangzhou.personal.cr.aliyuncs.com/passionke/claw-tap:latest",
        )
        self.assertIn("crpi-", template_gateway_worker_image())

    def test_global_default(self) -> None:
        self.assertFalse(region_is_china())
        self.assertEqual(template_debian_base_image(), "debian:bookworm-slim")
        self.assertEqual(template_debian_apt_mirror(), "")
        self.assertEqual(template_claude_tap_image(), "ghcr.io/passionke/claw-tap:v0.0.11")
        self.assertEqual(
            template_gateway_worker_image(),
            "ghcr.io/passionke/claw-gateway-worker:release-v1.6.17",
        )

    def test_ci_skips_cn_mirror(self) -> None:
        os.environ["region"] = "china"
        os.environ["GITHUB_ACTIONS"] = "true"
        self.assertFalse(cn_mirror_enabled())
        self.assertEqual(template_debian_base_image(), "debian:bookworm-slim")

    def test_china_uses_acr_even_with_claw_use_docker_io(self) -> None:
        os.environ["region"] = "china"
        os.environ["CLAW_USE_DOCKER_IO"] = "1"
        self.assertEqual(template_debian_base_image(), "debian:bookworm-slim")
        self.assertIn("crpi-", template_claude_tap_image())
        self.assertIn("crpi-", template_gateway_worker_image())

    def test_region_from_bashrc(self) -> None:
        with patch(
            "e2b_template_registry._region_from_bashrc",
            return_value="china",
        ):
            self.assertEqual(region_name(), "china")


if __name__ == "__main__":
    unittest.main()

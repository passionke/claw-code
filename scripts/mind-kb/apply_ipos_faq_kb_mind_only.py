#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""Compatibility shim; use apploy_faq_kb_mind_only.py instead. Author: kejiqing"""

from __future__ import annotations

import runpy
from pathlib import Path


if __name__ == "__main__":
    target = Path(__file__).with_name("apploy_faq_kb_mind_only.py")
    runpy.run_path(str(target), run_name="__main__")

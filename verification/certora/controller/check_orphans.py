#!/usr/bin/env python3
"""Compatibility wrapper for the repository-wide Certora orphan check."""

import runpy
from pathlib import Path

ROOT_CHECK = Path(__file__).resolve().parents[1] / "check_orphans.py"
runpy.run_path(str(ROOT_CHECK), run_name="__main__")

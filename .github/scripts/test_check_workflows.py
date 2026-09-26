#!/usr/bin/env python3
"""Self-test for the fork-guard rule in check_workflows.py.

Usage: python3 .github/scripts/test_check_workflows.py
"""
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import check_workflows as check  # noqa: E402

G = check.FORK_GUARD
CASES = {
    "inline trigger list": ("on: [push, pull_request]\njobs:\n  a:\n    runs-on: self-hosted\n", 1),
    "scalar trigger": ("on: pull_request\njobs:\n  a:\n    runs-on: [self-hosted, linux]\n", 1),
    "runs-on list": ("on:\n  pull_request:\njobs:\n  a:\n    runs-on:\n      - self-hosted\n      - linux\n", 1),
    "custom runner label": ("on:\n  pull_request:\njobs:\n  a:\n    runs-on: my-runner\n", 1),
    "hosted runner": ("on:\n  pull_request:\njobs:\n  a:\n    runs-on: ubuntu-latest\n", 0),
    "guard alternative": (f"on:\n  pull_request:\njobs:\n  a:\n    if: github.event_name != 'pull_request' || {G}\n    runs-on: self-hosted\n", 0),
    "guard bypassed by or": (f"on:\n  pull_request:\njobs:\n  a:\n    if: always() || {G}\n    runs-on: self-hosted\n", 1),
    "guard conjunct in expression": ("on:\n  pull_request:\njobs:\n  a:\n    if: ${{ github.event_name == 'pull_request' && " + G + " }}\n    runs-on: self-hosted\n", 0),
    "schedule or dispatch only": ("on:\n  pull_request:\n  schedule:\njobs:\n  a:\n    if: >-\n      (github.event_name == 'schedule' &&\n      startsWith(github.event.schedule, '0 2 ')) ||\n      (github.event_name == 'workflow_dispatch' &&\n      (inputs.suite == 'mutants' || inputs.suite == 'all'))\n    runs-on: self-hosted\n", 0),
    "event or free condition": ("on:\n  pull_request:\njobs:\n  a:\n    if: github.event_name == 'schedule' || inputs.x == 'y'\n    runs-on: self-hosted\n", 1),
    "pull_request outside on": ("on:\n  workflow_dispatch:\nconcurrency:\n  cancel-in-progress: ${{ github.event_name == 'pull_request' }}\njobs:\n  a:\n    runs-on: self-hosted\n", 0),
}

failures = [
    name for name, (text, want) in CASES.items()
    if len(check.fork_guard_errors("t.yml", text)) != want
]
for name in failures:
    print(f"fork-guard self-test failed: {name}")
sys.exit(1 if failures else 0)

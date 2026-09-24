#!/usr/bin/env python3
"""Fail when a workflow action is not pinned to a full commit SHA, or when the
workflow that runs the Static Gates job has a paths filter.

Usage: python3 .github/scripts/check_workflows.py
"""
import re
import sys
from pathlib import Path

WORKFLOWS = Path(__file__).resolve().parents[1] / "workflows"
USES = re.compile(r"^\s*(?:-\s+)?uses:\s*['\"]?([^\s'\"#]+)")
PINNED = re.compile(r"[^@]+@[0-9a-f]{40}")
PATHS = re.compile(r"^\s+paths(?:-ignore)?:", re.MULTILINE)
STATIC_GATES = re.compile(r"^\s+name:\s*['\"]?Static Gates['\"]?\s*$", re.MULTILINE)


def main() -> int:
    errors = []
    gates = []
    for wf in sorted(WORKFLOWS.glob("*.y*ml")):
        text = wf.read_text()
        for n, line in enumerate(text.splitlines(), 1):
            m = USES.match(line)
            if m and not m.group(1).startswith(("./", "docker://")) and not PINNED.fullmatch(m.group(1)):
                errors.append(f"{wf.name}:{n}: {m.group(1)} is not pinned to a full commit SHA")
        if STATIC_GATES.search(text):
            gates.append(wf.name)
            if PATHS.search(text):
                errors.append(f"{wf.name}: runs Static Gates, so it must not have a paths filter")
    if not gates:
        errors.append("no workflow defines the Static Gates job")
    for e in errors:
        print(e)
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main())

#!/usr/bin/env python3
"""Check that every conf-listed rule name has a matching #[rule] fn in spec.

Run from stellar/: python3 controller/certora/check_orphans.py
Exits non-zero on any orphan; prints the pairs.
"""
import re
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent  # stellar/controller
SPEC = ROOT / "certora" / "spec"
CONFS = ROOT / "confs"

source_rules = set()
for f in SPEC.glob("*_rules.rs"):
    source_rules.update(
        re.findall(r"#\[rule\]\s*(?:pub\s+)?fn\s+(\w+)", f.read_text())
    )

orphans = []
for c in sorted(CONFS.glob("*.conf")):
    d = json.loads(c.read_text())
    for r in d.get("rule", []):
        if r not in source_rules:
            orphans.append((c.name, r))

if orphans:
    print("Orphan conf entries (listed in conf but no matching #[rule] in spec):")
    for c, r in orphans:
        print(f"  {c}: {r}")
    sys.exit(1)

conf_count = len(list(CONFS.glob("*.conf")))
print(f"OK: {conf_count} confs, {len(source_rules)} source rules, zero orphans")

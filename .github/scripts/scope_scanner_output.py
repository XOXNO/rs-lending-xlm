"""Narrow soroban-scanner JSON to deployable crates.

The scanner walks the whole workspace so the symbol resolver can follow
cross-crate refs; test/fuzz findings are noise for the audit gate.
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
IN_SCOPE = ("common/src/",) + tuple(
    f"{manifest.parent.relative_to(ROOT)}/src/"
    for group in ("contracts", "interfaces")
    for manifest in (ROOT / group).glob("*/Cargo.toml")
)

def in_scope(path: str) -> bool:
    try:
        relative = (ROOT / path).resolve().relative_to(ROOT).as_posix()
    except ValueError:
        return False
    return relative.startswith(IN_SCOPE)

def main() -> None:
    data = json.load(sys.stdin)
    if not isinstance(data, dict) or not isinstance(data.get("scanned"), list) or not isinstance(data.get("detector_responses"), dict):
        raise ValueError("Invalid scanner report: expected scanned list and detector_responses object")
    data["scanned"] = sorted(p for p in data["scanned"] if in_scope(p))
    if not data["scanned"]:
        raise ValueError("Scanner report contains no in-scope scanned files")

    narrowed: dict = {}
    for name, payload in data["detector_responses"].items():
        if not isinstance(payload["findings"], list):
            raise ValueError("Invalid scanner report: expected findings list")
        findings = []
        for finding in payload["findings"]:
            if not isinstance(finding["instances"], list):
                raise ValueError("Invalid scanner report: expected instances list")
            kept = [
                i for i in finding["instances"] if in_scope(i["path"])
            ]
            if kept:
                findings.append({**finding, "instances": kept})
        if findings:
            narrowed[name] = {**payload, "findings": findings}
    data["detector_responses"] = narrowed
    json.dump(data, sys.stdout, indent=2)
    sys.stdout.write("\n")

if __name__ == "__main__":
    main()

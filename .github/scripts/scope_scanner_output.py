#!/usr/bin/env python3
"""Narrow soroban-scanner JSON output to the deployable crate surface.

The scanner has to see the whole workspace (verification/test-harness,
verification/fuzz, …) so
its symbol resolver can follow cross-crate references without recursing
unboundedly — but findings from test or fuzz code are noise for an audit
gate. This filter keeps only findings whose file path is under one of
`common/src/`, `pool/src/`, `pool-interface/src/`, or `controller/src/`.
"""
from __future__ import annotations

import json
import sys

IN_SCOPE_MARKERS = (
    "/common/src/",
    "/pool/src/",
    "/pool-interface/src/",
    "/controller/src/",
)


def in_scope(path: str) -> bool:
    return any(marker in path for marker in IN_SCOPE_MARKERS)


def main() -> None:
    data = json.load(sys.stdin)
    data["scanned"] = sorted(p for p in data.get("scanned", []) if in_scope(p))

    narrowed = {}
    for detector_name, payload in data.get("detector_responses", {}).items():
        if not isinstance(payload, dict):
            continue
        findings = []
        for finding in payload.get("findings", []):
            kept = [
                inst
                for inst in finding.get("instances", [])
                if in_scope(inst.get("path", ""))
            ]
            if kept:
                findings.append({**finding, "instances": kept})
        if findings:
            narrowed[detector_name] = {**payload, "findings": findings}
    data["detector_responses"] = narrowed

    json.dump(data, sys.stdout, indent=2)
    sys.stdout.write("\n")


if __name__ == "__main__":
    main()

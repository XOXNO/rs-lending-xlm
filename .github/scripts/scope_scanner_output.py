
"""Narrow soroban-scanner JSON to deployable crates.

The scanner walks the whole workspace so the symbol resolver can follow
cross-crate refs; test/fuzz findings are noise for the audit gate.
"""

from __future__ import annotations

import json
import sys

IN_SCOPE = (
    "/common/src/",
    "/contracts/pool/",
    "/contracts/controller/",
    "/contracts/governance/",
    "/contracts/price-aggregator/",
    "/contracts/defindex-strategy/",
    "/interfaces/pool/",
    "/interfaces/controller/",
    "/interfaces/governance/",
    "/interfaces/price-aggregator/",
)


def in_scope(path: str) -> bool:
    return any(m in path for m in IN_SCOPE)


def main() -> None:
    data = json.load(sys.stdin)
    data["scanned"] = sorted(p for p in data.get("scanned", []) if in_scope(p))

    narrowed: dict = {}
    for name, payload in data.get("detector_responses", {}).items():
        if not isinstance(payload, dict):
            continue
        findings = []
        for finding in payload.get("findings", []):
            kept = [
                i for i in finding.get("instances", []) if in_scope(i.get("path", ""))
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

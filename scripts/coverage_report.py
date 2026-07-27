#!/usr/bin/env python3
"""Summarize llvm-cov LCOV into a markdown table for one coverage mode."""

from __future__ import annotations

import sys
from pathlib import Path

REPO_MARKER = "/rs-lending-xlm/"

# path substring → included when mode matches
MODE_PATHS: dict[str, tuple[str, ...]] = {
    "controller": ("/contracts/controller/", "/common/"),
    "pool": ("/contracts/pool/",),
    "price-aggregator": ("/contracts/price-aggregator/", "/common/"),
    "merged": (
        "/contracts/controller/",
        "/contracts/pool/",
        "/contracts/price-aggregator/",
        "/common/",
    ),
}


def parse_lcov(path: Path) -> dict[str, dict[str, int]]:
    files: dict[str, dict[str, int]] = {}
    current: str | None = None
    for raw in path.read_text().splitlines():
        if raw.startswith("SF:"):
            current = raw[3:]
        elif raw.startswith("LH:") and current:
            files.setdefault(current, {})["hit"] = int(raw[3:])
        elif raw.startswith("LF:") and current:
            files.setdefault(current, {})["total"] = int(raw[3:])
    return files


def keep(path: str, mode: str) -> bool:
    if REPO_MARKER not in path or "/tests/test-harness/" in path:
        return False
    prefixes = MODE_PATHS.get(mode)
    if prefixes is None:
        raise ValueError(f"unsupported mode: {mode}")
    return any(p in path for p in prefixes)


def write_report(lcov_path: Path, report_path: Path, mode: str) -> tuple[int, int, float]:
    selected = {k: v for k, v in parse_lcov(lcov_path).items() if keep(k, mode)}
    hit_total = total_total = 0
    lines = [
        f"# Stellar Lending Protocol — {mode.capitalize()} Coverage Report",
        "",
        "| File | Lines | Hit | Miss | Coverage |",
        "|------|-------|-----|------|----------|",
    ]
    for path in sorted(selected):
        hit = selected[path].get("hit", 0)
        total = selected[path].get("total", 0)
        pct = (hit / total * 100) if total else 0.0
        short = path.split(REPO_MARKER)[-1]
        lines.append(f"| {short} | {total} | {hit} | {total - hit} | {pct:.1f}% |")
        hit_total += hit
        total_total += total
    overall = (hit_total / total_total * 100) if total_total else 0.0
    lines.append(
        f"| **TOTAL** | **{total_total}** | **{hit_total}** | "
        f"**{total_total - hit_total}** | **{overall:.1f}%** |"
    )
    report_path.write_text("\n".join(lines) + "\n")
    return hit_total, total_total, overall


def main() -> int:
    if len(sys.argv) != 4:
        modes = "|".join(MODE_PATHS)
        print(
            f"usage: coverage_report.py <lcov_path> <report_path> <{modes}>",
            file=sys.stderr,
        )
        return 1
    hit, total, overall = write_report(Path(sys.argv[1]), Path(sys.argv[2]), sys.argv[3])
    print(f"TOTAL {hit}/{total} {overall:.2f}%")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

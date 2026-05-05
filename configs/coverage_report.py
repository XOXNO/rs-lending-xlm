#!/usr/bin/env python3

from __future__ import annotations

import sys
from pathlib import Path


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


REPO_MARKER = "/rs-lending-xlm/"


def keep(path: str, mode: str) -> bool:
    if REPO_MARKER not in path or "/verification/test-harness/" in path:
        return False
    if mode == "controller":
        return "/controller/" in path or "/common/" in path
    if mode == "pool":
        return "/pool/" in path
    if mode == "merged":
        return "/controller/" in path or "/common/" in path or "/pool/" in path
    raise ValueError(f"unsupported mode: {mode}")


def write_report(lcov_path: Path, report_path: Path, mode: str) -> tuple[int, int, float]:
    files = parse_lcov(lcov_path)
    selected = {k: v for k, v in files.items() if keep(k, mode)}

    hit_total = 0
    total_total = 0
    lines = [
        f"# Stellar Lending Protocol — {mode.capitalize()} Coverage Report",
        "",
        "| File | Lines | Hit | Miss | Coverage |",
        "|------|-------|-----|------|----------|",
    ]

    for path in sorted(selected):
        data = selected[path]
        hit = data.get("hit", 0)
        total = data.get("total", 0)
        miss = total - hit
        pct = (hit / total * 100) if total else 0
        short = path.split(REPO_MARKER)[-1]
        lines.append(f"| {short} | {total} | {hit} | {miss} | {pct:.1f}% |")
        hit_total += hit
        total_total += total

    overall = (hit_total / total_total * 100) if total_total else 0.0
    lines.append(
        f"| **TOTAL** | **{total_total}** | **{hit_total}** | **{total_total-hit_total}** | **{overall:.1f}%** |"
    )
    report_path.write_text("\n".join(lines) + "\n")
    return hit_total, total_total, overall


def main() -> int:
    if len(sys.argv) != 4:
        print("usage: coverage_report.py <lcov_path> <report_path> <controller|pool|merged>", file=sys.stderr)
        return 1

    lcov_path = Path(sys.argv[1])
    report_path = Path(sys.argv[2])
    mode = sys.argv[3]

    hit_total, total_total, overall = write_report(lcov_path, report_path, mode)
    print(f"TOTAL {hit_total}/{total_total} {overall:.2f}%")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

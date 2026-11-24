"""Summarize nightly Rust branch coverage from an llvm-cov JSON export."""

from __future__ import annotations

import json
import sys
from collections import defaultdict
from pathlib import Path

from coverage_report import keep, source_path


def write_report(json_path: Path, report_path: Path, mode: str) -> tuple[int, int, float]:
    data = json.loads(json_path.read_text())
    runs = data.get("data")
    if not isinstance(runs, list) or len(runs) != 1:
        raise ValueError("expected one LLVM coverage data set")
    selected = [
        item for item in runs[0].get("files", [])
        if isinstance(item, dict) and keep(item.get("filename", ""), mode)
    ]
    if not selected:
        raise ValueError(f"no source files selected for {mode}")

    rows = []
    covered_total = branch_total = llvm_covered_total = llvm_branch_total = 0
    for item in sorted(selected, key=lambda value: source_path(value["filename"])):
        summary = item["summary"]["branches"]
        llvm_total = summary["count"]
        llvm_covered = summary["covered"]
        if not isinstance(llvm_total, int) or not isinstance(llvm_covered, int) or not 0 <= llvm_covered <= llvm_total:
            raise ValueError("invalid LLVM branch counts")
        # The JSON repeats source branch locations for compiled instantiations.
        # Combine their edge counts to measure each physical condition once.
        outcomes: dict[tuple[int, ...], list[int]] = defaultdict(lambda: [0, 0])
        for branch in item.get("branches", []):
            if len(branch) != 9 or branch[4] < 0 or branch[5] < 0:
                raise ValueError("invalid LLVM branch record")
            key = tuple(branch[:4] + branch[6:])
            outcomes[key][0] += branch[4]
            outcomes[key][1] += branch[5]
        total = 2 * len(outcomes)
        covered = sum(count > 0 for pair in outcomes.values() for count in pair)
        if total != llvm_total:
            raise ValueError("LLVM branch summary disagrees with physical branch sites")
        if total == 0:
            continue
        missed_lines = sorted({
            location[0] for location, counts in outcomes.items() if 0 in counts
        })
        rows.append((
            source_path(item["filename"]), covered, total, llvm_covered,
            missed_lines,
        ))
        covered_total += covered
        branch_total += total
        llvm_covered_total += llvm_covered
        llvm_branch_total += llvm_total
    if branch_total == 0:
        raise ValueError("branch coverage instrumentation produced zero branches")

    percent = covered_total / branch_total * 100
    lines = [
        f"# Stellar Lending Protocol — {mode.capitalize()} Rust Branch Coverage",
        "",
        "Experimental nightly Rust branch instrumentation. Repeated compiled "
        "instantiations of each source condition are combined. LLVM's file "
        "summary is retained for comparison. This metric does not prove every "
        "semantic path. The line list shows conditions with a zero-count outcome.",
        "",
        "| File | Branch outcomes | Covered | Missed | Coverage | LLVM missed | Difference | Missed condition lines |",
        "|---|---:|---:|---:|---:|---:|---:|---|",
    ]
    for path, covered, total, llvm_covered, missed_lines in rows:
        pct = covered / total * 100
        sites = ", ".join(map(str, missed_lines)) or "—"
        llvm_missed = total - llvm_covered
        difference = llvm_missed - (total - covered)
        lines.append(
            f"| {path} | {total} | {covered} | {total - covered} | {pct:.1f}% | "
            f"{llvm_missed} | {difference:+d} | {sites} |"
        )
    lines.append(
        f"| **TOTAL** | **{branch_total}** | **{covered_total}** | "
        f"**{branch_total - covered_total}** | **{percent:.1f}%** | "
        f"**{llvm_branch_total - llvm_covered_total}** | "
        f"**{covered_total - llvm_covered_total:+d}** | |"
    )
    lines.extend([
        "",
        f"LLVM file-summary metric: {llvm_covered_total}/{llvm_branch_total} "
        f"({llvm_covered_total / llvm_branch_total * 100:.2f}%).",
    ])
    report_path.write_text("\n".join(lines) + "\n")
    return covered_total, branch_total, percent


def main() -> int:
    if len(sys.argv) != 4:
        print(
            "usage: branch_coverage_report.py <json_path> <report_path> <mode>",
            file=sys.stderr,
        )
        return 1
    covered, total, percent = write_report(
        Path(sys.argv[1]), Path(sys.argv[2]), sys.argv[3]
    )
    print(f"BRANCHES {covered}/{total} {percent:.2f}%")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

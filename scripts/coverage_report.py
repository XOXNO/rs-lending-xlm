"""Summarize llvm-cov LCOV into a markdown table for one coverage mode."""

from __future__ import annotations

import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
MODE_PATHS: dict[str, tuple[str, ...]] = {
    "controller": ("/contracts/controller/", "/common/"),
    "pool": ("/contracts/pool/",),
    "price-aggregator": ("/contracts/price-aggregator/", "/common/"),
    # Every deployable contract under contracts/, plus common. Keep in step with
    # the package list in the Makefile's `coverage-merged` target: a package
    # measured there but not matched here is silently dropped from the report.
    "merged": (
        "/contracts/controller/",
        "/contracts/pool/",
        "/contracts/price-aggregator/",
        "/contracts/governance/",
        "/contracts/swap-aggregator/",
        "/contracts/xoxno-oracle/",
        "/contracts/position-nft/",
        "/contracts/defindex-strategy/",
        "/contracts/curator/",
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


def parse_source_lines(path: Path) -> dict[str, dict[int, int]]:
    """Collect each physical source line once, even across compiled instances."""
    files: dict[str, dict[int, int]] = {}
    current: str | None = None
    for raw in path.read_text().splitlines():
        if raw.startswith("SF:"):
            current = raw[3:]
        elif raw == "end_of_record":
            current = None
        elif raw.startswith("DA:") and current:
            number, hits, *_ = raw[3:].split(",")
            line_number = int(number)
            hit_count = int(hits)
            if line_number <= 0 or hit_count < 0:
                raise ValueError("invalid LCOV source line")
            counts = files.setdefault(current, {})
            counts[line_number] = max(counts.get(line_number, 0), hit_count)
    return files


def source_path(path: str) -> str:
    source = Path(path.replace("\\", "/"))
    if ".." in source.parts:
        return ""
    if source.is_absolute():
        try:
            relative = source.relative_to(ROOT).as_posix()
        except ValueError:
            # Cached baselines can come from another checkout of this repository.
            _, found, relative = source.as_posix().rpartition(f"/{ROOT.name}/")
            if not found or {"vendor", "mock", ".cargo"}.intersection(source.parts):
                return ""
    else:
        relative = source.as_posix()
    if any(relative.startswith(p.strip("/") + "/src/") for p in MODE_PATHS["merged"]):
        return relative
    return ""


def keep(path: str, mode: str) -> bool:
    prefixes = MODE_PATHS.get(mode)
    if prefixes is None:
        raise ValueError(f"unsupported mode: {mode}")
    return any(("/" + source_path(path)).startswith(p) for p in prefixes)

def write_report(lcov_path: Path, report_path: Path, mode: str) -> tuple[int, int, float]:
    selected = {k: v for k, v in parse_lcov(lcov_path).items() if keep(k, mode)}
    source_lines = parse_source_lines(lcov_path)
    if not selected or not any(source_lines.get(path) for path in selected):
        raise ValueError(f"no measured source lines for {mode}")
    if any(set(v) != {"hit", "total"} or not 0 <= v["hit"] <= v["total"] for v in selected.values()):
        raise ValueError("invalid LCOV line counts")
    if any(not source_lines.get(path) for path in selected):
        raise ValueError("LCOV file summary has no source-line entries")
    hit_total = total_total = summary_hit = summary_total = 0
    lines = [
        f"# Stellar Lending Protocol — {mode.capitalize()} Coverage Report",
        "",
        "Physical native source-line coverage from LCOV `DA` entries. Each line is "
        "counted once across compiled instances. Includes test-only code under `src/`; "
        "does not measure assertion strength, Rust branches, or production WASM execution.",
        "The final two columns retain LLVM's `LF/LH` file-summary missed-line count "
        "and its difference from zero-hit physical lines.",
        "",
        "| File | Lines | Hit | Miss | Coverage | LLVM missed | Difference |",
        "|------|-------|-----|------|----------|-------------|------------|",
    ]
    for path in sorted(selected):
        counts = source_lines[path]
        hit = sum(count > 0 for count in counts.values())
        total = len(counts)
        pct = (hit / total * 100) if total else 0.0
        llvm_missed = selected[path]["total"] - selected[path]["hit"]
        difference = llvm_missed - (total - hit)
        short = source_path(path)
        lines.append(
            f"| {short} | {total} | {hit} | {total - hit} | {pct:.1f}% | "
            f"{llvm_missed} | {difference:+d} |"
        )
        hit_total += hit
        total_total += total
        summary_hit += selected[path]["hit"]
        summary_total += selected[path]["total"]
    overall = (hit_total / total_total * 100) if total_total else 0.0
    lines.append(
        f"| **TOTAL** | **{total_total}** | **{hit_total}** | "
        f"**{total_total - hit_total}** | **{overall:.1f}%** | "
        f"**{summary_total - summary_hit}** | "
        f"**{summary_total - summary_hit - (total_total - hit_total):+d}** |"
    )
    lines.extend([
        "",
        f"LLVM file-summary metric: {summary_hit}/{summary_total} "
        f"({summary_hit / summary_total * 100:.2f}%). "
        "It may count additional regions or compiled instances that do not map "
        "to distinct zero-hit source lines.",
        "",
        "## Zero-hit physical source lines",
        "",
        "| File | Lines |",
        "|---|---|",
    ])
    for path in sorted(selected):
        missed = sorted(
            number for number, count in source_lines[path].items() if count == 0
        )
        if missed:
            lines.append(
                f"| {source_path(path)} | {', '.join(map(str, missed))} |"
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

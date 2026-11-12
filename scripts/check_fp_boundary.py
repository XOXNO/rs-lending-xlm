#!/usr/bin/env python3
"""Keep raw fixed-point helpers and unreviewed wide math out of production callers."""

import re
from pathlib import Path

from check_access_control import strip_comments

ROOT = Path(__file__).resolve().parents[1]
RAW = re.compile(
    r"\b(?:fp_core|(?:try_)?mul_div_(?:half_up|floor|ceil)(?:_saturating)?"
    r"|rescale_half_up|div_by_int_half_up)\b"
    r"|\bmath\s*::[^;]*\breference\b"
)
WIDE = re.compile(r"\b[IU]256\b")
WIDE_FILES = {
    "common/src/oracle/lp.rs": "integer square root and wide LP valuation",
    "common/src/oracle/lp_stable.rs": "StableSwap invariant and wide LP valuation",
    "common/src/oracle/observation.rs": "checked oracle wire-value conversion",
    "common/src/oracle/providers/chainlink.rs": "oracle wire type",
    "common/src/oracle/providers/redstone.rs": "oracle wire type",
    "contracts/swap-aggregator/src/venues/soroswap.rs": "integer AMM reserve formula",
    "contracts/swap-aggregator/src/venues/sushi.rs": "pool wire type for sqrt-price limits",
    "contracts/xoxno-oracle/src/aggregation.rs": "oracle wire type",
}


def violations(relative: str, source: str) -> list[str]:
    if relative in {"common/src/math/fp.rs", "common/src/math/fp_core.rs", "common/src/math/mod.rs"}:
        return []
    code = strip_comments(source, literals=True)
    errors = []
    for pattern, message in (
        (RAW, "use the typed common::math::fp API"),
        (WIDE, "wide math needs an explicit domain exception"),
    ):
        if pattern is WIDE and relative in WIDE_FILES:
            continue
        for match in pattern.finditer(code):
            line = code.count("\n", 0, match.start()) + 1
            errors.append(f"{relative}:{line}: {message}")
    return errors


def main() -> int:
    sources = [ROOT / "common/src", *(ROOT / "contracts").glob("*/src"), *(ROOT / "interfaces").glob("*/src")]
    errors = []
    for directory in sources:
        for file in sorted(directory.rglob("*.rs")):
            errors.extend(violations(file.relative_to(ROOT).as_posix(), file.read_text()))
    if errors:
        print("\n".join(errors))
        return 1
    print("Fixed-point boundary: OK")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

#!/usr/bin/env python3
"""Keep raw fixed-point helpers and unreviewed wide math out of production callers."""

import re
from collections import Counter
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

# Every production controller extraction from a typed fixed-point value. The
# key is the stripped source line and the value is its reviewed occurrence
# count in that file. Exact counts make both new escapes and stale approvals
# fail closed. These are serialization boundaries only: ABI/view returns,
# raw storage/interface structs, events, or external call arguments.
CONTROLLER_RAW_BOUNDARIES: dict[str, Counter[str]] = {
    "contracts/controller/src/events/mod.rs": Counter(
        {
            "position.scaled_amount.raw(),": 2,
            "position.liquidation_threshold.raw() as u32,": 1,
            "position.liquidation_bonus.raw() as u32,": 1,
            "position.loan_to_value.raw() as u32,": 1,
            "position.liquidation_fees.raw() as u32,": 1,
        }
    ),
    "contracts/controller/src/risk/params.rs": Counter(
        {"market_index.supply_index.raw(),": 2}
    ),
    "contracts/controller/src/views.rs": Counter(
        {
            "health_factor_wad(env, account_id).raw()": 1,
            ".raw()": 2,
            "supply_index: index.supply_index.raw(),": 1,
            "borrow_index: index.borrow_index.raw(),": 1,
            "(entry.scaled_amount, fee_scaled.raw())": 1,
            "risk::sum_debt_usd(env, &mut cache, &borrow).raw()": 1,
            "risk::calculate_ltv_collateral_wad(env, &mut cache, &account.supply_positions).raw()": 1,
        }
    ),
    "contracts/controller/src/spoke_usage.rs": Counter(
        {"side.set_scaled(&mut usage, next.raw());": 2}
    ),
    "contracts/controller/src/strategies/legs.rs": Counter(
        {
            "scaled_amount: supply_position.scaled_amount.raw(),": 1,
            "scaled_amount: debt_position.scaled_amount.raw(),": 1,
        }
    ),
    "contracts/controller/src/positions/liquidation/bad_debt.rs": Counter(
        {
            "total_borrow_usd_wad: totals.total_debt.raw(),": 1,
            "total_collateral_usd_wad: totals.total_collateral.raw(),": 1,
        }
    ),
    "contracts/controller/src/positions/liquidation/math.rs": Counter(
        {
            "max_debt_usd: self.repayment.repay_usd.raw(),": 1,
            "bonus_bps: self.repayment.bonus.raw(),": 1,
            "usd_wad: payment_usd.raw(),": 1,
            "scaled_amount: seized_scaled.raw(),": 1,
            "bonus_scaled: bonus_scaled.raw(),": 1,
            "liquidation_fees: position.liquidation_fees.raw() as u32,": 1,
            "scaled_amount: scaled_amount.raw(),": 1,
            ".raw(),": 1,
            "usd_wad: new_usd.raw(),": 1,
        }
    ),
    "contracts/controller/src/positions/liquidation/apply.rs": Counter(
        {"scaled_amount: fee_scaled.raw(),": 1}
    ),
    "contracts/controller/src/positions/liquidation/mod.rs": Counter(
        {"repaid_usd_wad: received_usd.raw(),": 1}
    ),
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

    if relative.startswith("contracts/controller/src/"):
        approved = CONTROLLER_RAW_BOUNDARIES.get(relative, Counter()).copy()
        for line_number, line_source in enumerate(code.splitlines(), 1):
            if not re.search(r"\.raw\s*\(\s*\)", line_source):
                continue
            source_key = line_source.strip()
            if approved[source_key] > 0:
                approved[source_key] -= 1
                continue
            errors.append(
                f"{relative}:{line_number}: raw fixed-point extraction is allowed only "
                "at a reviewed serialization boundary"
            )
        for source_key, remaining in approved.items():
            if remaining > 0:
                errors.append(
                    f"{relative}: stale reviewed raw boundary ({remaining} missing): {source_key}"
                )
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

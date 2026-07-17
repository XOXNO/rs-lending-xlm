# ADR 0012: Per-Spoke Liquidation Curve

- Status: Accepted
- Date: 2026-07-08
- Deciders: XOXNO Lending contract team

## Context

A flat per-asset `liquidation_bonus_bps` mis-pays both ends: barely underwater
accounts pay full bonus; deep underwater accounts get no extra incentive when
urgency matters. Spokes also need different aggressiveness (e.g. correlated /
stable collateral vs volatile).

## Decision

Each spoke stores a three-parameter curve (defaults stamped at spoke creation
in `contracts/controller/src/constants.rs` / `config/spoke.rs`):

| Field | Meaning | Default |
|-------|---------|---------|
| `liquidation_target_hf_wad` | HF a liquidation restores to; repay sized/capped to this | 1.10 WAD (`DEFAULT_LIQUIDATION_TARGET_HF_WAD`) |
| `hf_for_max_bonus_wad` | HF at or below which max bonus applies | 0.80 WAD (`DEFAULT_HF_FOR_MAX_BONUS_WAD`) |
| `liquidation_bonus_factor_bps` | Scales the **increment** above the asset base bonus | `10_000` (`DEFAULT_LIQUIDATION_BONUS_FACTOR_BPS`, 1.0×) |

Runtime (`LiquidationCurve` in `positions/liquidation/math.rs`):

```text
bonus = base + factor × (max − base) × min(1, (target − hf) / (target − hf_for_max))
```

Base and max come from the account’s collateral mix (listing bonus and
threshold-derived max), not from the spoke curve alone. Listing product ceiling
(`liquidation_threshold_bps * (BPS + bonus_bps) <= BPS * BPS`) still constrains
configured bonuses.

Validation (`validate_liquidation_curve`):

- `WAD < target_hf_wad <= MAX_LIQUIDATION_TARGET_HF_WAD` (10 WAD)  
- `0 < hf_for_max_bonus_wad < target_hf_wad`  
- `bonus_factor_bps <= BPS`  

Governance: `AdminOperation::SetSpokeLiquidationCurve` (timelocked).

## Alternatives considered

- Flat per-asset bonus only — remains the curve floor; cannot express depth.  
- One global curve — spokes exist to segment risk.  
- Per-position bonus snapshots — storage churn.  
- Auction liquidation — different protocol generation.  

## Consequences

**Positive:** default bonus factor 1.0× keeps the asset base bonus as the floor;
small seizures near target HF; deeper positions pay a larger bounded bonus; one
timelocked operation per spoke.

**Costs:** three more listing parameters; liquidator tooling must read the
spoke curve.

## References

- `common/src/types/controller.rs`, `common/src/validation.rs`  
- `contracts/controller/src/constants.rs`  
- `contracts/controller/src/positions/liquidation/math.rs`  
- `contracts/governance/src/op.rs`  
- [INVARIANTS.md](../INVARIANTS.md) §3.3, §4.1  

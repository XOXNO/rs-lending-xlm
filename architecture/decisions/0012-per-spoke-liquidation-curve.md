# ADR 0012: Per-Spoke Liquidation Curve

- Status: Accepted
- Date: 2026-07-08
- Deciders: XOXNO Lending contract team
- Supersedes: none

## Context

The original liquidation model applied each collateral asset's flat
`liquidation_bonus_bps` regardless of how far underwater the account was. A
flat bonus is miscalibrated at both ends: a barely-underwater account pays the
full bonus (over-penalizing the borrower and over-paying the liquidator for a
low-risk close), while a deeply-underwater account offers no extra incentive
exactly when liquidator urgency matters most for avoiding bad debt.

Different spokes also need different liquidation aggressiveness: a
correlated-asset or RWA spoke can restore health with smaller seizures than a
volatile-collateral spoke.

## Decision

Give every spoke a three-parameter liquidation curve, stored on the spoke
config and stamped with defaults at spoke creation so storage always carries
effective values:

- `liquidation_target_hf_wad` — the health factor a liquidation restores the
  account to. Repayment is sized to reach this target; a requested repayment
  above the ideal amount is capped.
  Default `DEFAULT_LIQUIDATION_TARGET_HF_WAD = 1.02 WAD`.
- `hf_for_max_bonus_wad` — the health factor at or below which the maximum
  bonus applies. Default `= target / 2` (0.51 WAD).
- `liquidation_bonus_factor_bps` — scales the bonus increment between the
  base and the max. Default `10_000` (1.0x), which reproduces the pre-curve
  behavior byte-for-byte.

Runtime (`contracts/controller/src/positions/liquidation/math.rs`,
`LiquidationCurve`): the bonus scale is linear in health-factor depth —
`(target - hf) / (target - hf_for_max_bonus)`, clamped to `[0, 1]` — and the
resulting increment above the asset's base bonus is weighted by
`bonus_factor`. The per-asset seizure ceiling
(`liquidation_threshold_bps * (BPS + bonus_bps) <= BPS * BPS`) is unchanged
and still bounds the realized bonus.

Validation (`common/src/validation.rs::validate_liquidation_curve`):

- `WAD < target_hf_wad <= MAX_LIQUIDATION_TARGET_HF_WAD` (10 WAD), so an
  oversized target cannot overflow `target_hf * total_debt`.
- `0 < hf_for_max_bonus_wad < target_hf_wad`.
- `bonus_factor_bps <= BPS`, because the bonus increment is added without a
  re-clamp against the dynamic max; a factor above 1.0x could push the
  realized bonus past the seizure-safety ceiling.

Governance: curve changes ride the timelock as
`AdminOperation::SetSpokeLiquidationCurve(SpokeLiquidationCurveArgs)`
(`contracts/governance/src/op.rs`).

## Alternatives Considered

- **Keep the flat per-asset bonus.** Rejected as the only model; it remains
  the curve's floor. The flat bonus cannot express depth-dependent urgency or
  spoke-specific aggressiveness.
- **A single global curve.** Rejected: spokes exist precisely to segment risk
  profiles; a volatile-collateral spoke and an RWA spoke need different
  targets and bonus ramps.
- **Per-position dynamic bonus snapshots.** Rejected for now:
  snapshotting curve output per position adds storage and re-stamp churn; the
  spoke-level curve achieves depth sensitivity with zero per-position state.
- **Auction-based liquidation.** Rejected: a different protocol design with
  materially higher complexity and latency; out of scope for this protocol
  generation.

## Consequences

Positive:

- Default parameters are behavior-preserving, so the change shipped with no
  economic migration.
- Repayment capped at the ideal amount shrinks seizures for barely-underwater
  accounts; deeper accounts pay a larger, bounded bonus.
- Spoke operators tune liquidation economics through one timelocked operation.

Accepted costs:

- Three more governance-tunable parameters to review at listing time; the
  INVARIANTS §4.1 bounds and the timelock are the guardrails.
- Liquidator tooling must read the spoke curve to predict proceeds; a flat
  assumption now underestimates deep-liquidation bonuses.

## References

- `common/src/types/controller.rs` (`SpokeConfig` curve fields)
- `common/src/validation.rs::validate_liquidation_curve`
- `contracts/controller/src/constants.rs`
  (`DEFAULT_LIQUIDATION_TARGET_HF_WAD`, `DEFAULT_HF_FOR_MAX_BONUS_WAD`,
  `DEFAULT_LIQUIDATION_BONUS_FACTOR_BPS`)
- `contracts/controller/src/positions/liquidation/math.rs`
  (`LiquidationCurve`)
- `contracts/governance/src/op.rs`
  (`AdminOperation::SetSpokeLiquidationCurve`)
- [ADR 0011](./0011-pause-and-freeze-matrix.md),
  [INVARIANTS.md §4.1](../INVARIANTS.md)

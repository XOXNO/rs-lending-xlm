# ADR 0007: Bad-Debt Socialization With Supply-Index Floor

- Status: Accepted
- Date: 2026-05-05
- Deciders: XOXNO Lending contract team
- Supersedes: none

## Context

Liquidations cap repayment at actual debt and apply a bonus to the
liquidator. When a stressed account ends up with collateral so small
that the liquidation bonus alone exceeds the residual collateral value,
no rational liquidator will close out the rest of the debt. The
remaining debt is uncollectable.

The protocol has to decide what to do with these stranded debts:

1. Leave them on the books and hope they become collectable later.
2. Pause the affected market.
3. Socialize them across that market's suppliers by reducing the supply
   index.

Doing nothing leaves a divergence between scaled debt and recoverable
liquidity that grows over time and propagates into health-factor math.
Pausing penalizes all market activity for losses that have already
crystallized. Socialization explicitly distributes the loss across the
suppliers who underwrote the lending.

A second concern is numerical safety: dropping the supply index toward
zero would divide by zero in scaled-balance conversions and revenue
accrual.

## Decision

Socialize uncollectable debt by reducing the pool's `supply_index_ray`,
floored at `SUPPLY_INDEX_FLOOR_RAW`.

**Trigger** (`controller/src/positions/liquidation.rs::check_bad_debt_after_liquidation`):
After liquidation, if for an account
`collateral_usd_wad <= BAD_DEBT_USD_THRESHOLD` (5 USD WAD) and
`debt > collateral`, the controller invokes `seize_position(Borrow)` on
each remaining debt asset's pool.

**Reduction** (`pool/src/interest.rs::apply_bad_debt_to_supply_index`):

- `total_supplied_value = supplied * supply_index`.
- `capped = min(bad_debt, total_supplied_value)`.
- `reduction_factor = (total_supplied_value - capped) / total_supplied_value`.
- `new_supply_index = supply_index * reduction_factor`.
- If the candidate index falls below 1/10 of the prior index, emit
  `PoolInsolventEvent` for off-chain monitoring with
  `bad_debt_ratio_bps`, `old_supply_index_ray`, `new_supply_index_ray`.
- Final write floors at `SUPPLY_INDEX_FLOOR_RAW` (10^18 raw Ray =
  10^-9 decimal). Revenue accrual paths additionally short-circuit when
  `index <= floor` so a near-zero index cannot divide-by-zero.

**Standalone path**: `clean_bad_debt(account_id)` is a `KEEPER`-only
entrypoint for accounts whose bad-debt state needs to be applied
outside a liquidation event.

## Alternatives Considered

- **Auto-pause the market on bad debt.** Rejected: the loss has already
  occurred. Pausing penalizes future suppliers and borrowers and
  obscures the financial reality. The pool emits `PoolInsolventEvent`
  for monitoring; the owner can still pause manually if warranted.
- **Insurance fund instead of socialization.** Rejected for launch:
  requires a separate accounting surface and capital provisioning model.
  The current design uses the supply-side claim to express loss
  directly. Future revenue diverted from `claim_revenue` could fund a
  reserve in a later ADR.
- **No floor on the supply index.** Rejected: a very large bad-debt
  event could drive the index to or near zero, breaking
  `scaled * index / RAY` reconstructions and the revenue-accrual
  divisor.
- **Pro-rata socialization via per-account writes.** Rejected: would
  sweep every supply position at the moment of socialization, which is
  exactly the work the scaled-balance design (ADR 0002) avoids.

## Consequences

Positive:

- Loss attribution is explicit and immediate: suppliers see the index
  step down.
- `PoolInsolventEvent` gives operators a high-signal trigger for
  out-of-band action (pause, communication, root-cause).
- Floor preserves the numerical health of all downstream math.
- Per-account work stays at zero — index motion captures the
  socialization.

Negative / accepted costs:

- Suppliers carry the loss directly; this needs to be communicated as
  part of the protocol's user-facing risk disclosure.
- The `BAD_DEBT_USD_THRESHOLD = $5` heuristic for triggering
  socialization is a tunable; sensitivity should be reviewed in
  audit.
- The 90% single-step drop signal is informational; the pool does not
  self-pause.

## References

- `SCF_BUILD_ARCHITECTURE.md` §10.5 (Liquidation and Bad Debt), §15
  (Implemented Safety Checks).
- `pool/src/interest.rs::apply_bad_debt_to_supply_index`
- `controller/src/positions/liquidation.rs::check_bad_debt_after_liquidation`
- `common/src/constants.rs::{SUPPLY_INDEX_FLOOR_RAW, BAD_DEBT_USD_THRESHOLD}`
- `common/src/events.rs::PoolInsolventEvent`

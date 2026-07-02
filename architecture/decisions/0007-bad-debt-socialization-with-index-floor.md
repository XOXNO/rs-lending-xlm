# ADR 0007: Bad-Debt Socialization With Supply-Index Floor

- Status: Accepted
- Date: 2026-05-05
- Deciders: XOXNO Lending contract team
- Supersedes: none

## Context

Liquidations cap repayment at actual debt and apply a bonus to the liquidator.
A stressed account can retain collateral so small that the liquidation bonus
alone exceeds the residual collateral value. No rational liquidator will close
out the rest of the debt. The remaining debt is uncollectable.

The protocol has to decide what to do with these stranded debts:

1. Leave them on the books and hope they become collectable later.
2. Pause the affected market.
3. Socialize them across that market's suppliers by reducing the supply
   index.

Doing nothing leaves a divergence between scaled debt and recoverable
liquidity that grows over time and propagates into health-factor math.
Pausing penalizes all market activity for losses that have already
crystallized. Socialization distributes the loss across the
suppliers who underwrote the lending.

A second concern is numerical safety: dropping the supply index toward
zero would divide by zero in scaled-balance conversions and revenue
accrual.

## Decision

Socialize uncollectable debt by reducing the pool's `supply_index_ray`,
floored at `SUPPLY_INDEX_FLOOR_RAW`.

**Trigger** (`contracts/controller/src/positions/liquidation/apply.rs::check_bad_debt_after_liquidation`):
After a liquidation, the account is socializable when
`is_socializable_bad_debt` holds: `debt > collateral` and
`collateral_usd_wad <= BAD_DEBT_USD_THRESHOLD` (5 USD WAD)
(`contracts/controller/src/positions/liquidation/math.rs`). The controller
then runs `execute_bad_debt_cleanup`, which seizes **both** sides of the
account: each supply (`Deposit`) position and each debt (`Borrow`) position is
collected into one batch and passed to the central pool via
`pool.seize_positions(entries)` in a single cross-contract call. On completion it publishes
`CleanBadDebtEvent { account_id, total_borrow_usd_wad,
total_collateral_usd_wad }`
(`contracts/controller/src/events/debt.rs`) and removes the account.

**Reduction**: only the `Borrow`-side seizure moves the asset's index. On the pool's
`seize_positions` (`contracts/pool/src/lib.rs`), a `Deposit` seizure adds the
scaled amount to pool revenue (no index motion); a `Borrow` seizure unscales
the debt and calls
`contracts/pool/src/interest.rs::apply_bad_debt_to_supply_index`:

- `total_supplied_value = supplied * supply_index`.
- `capped = min(bad_debt, total_supplied_value)`.
- `reduction_factor = (total_supplied_value - capped) / total_supplied_value`.
- `new_supply_index = supply_index * reduction_factor`.
- Final write floors at `SUPPLY_INDEX_FLOOR_RAW` (defined `= WAD`; 10^18 raw
  Ray = 10^-9 decimal). Revenue accrual paths short-circuit when
  `index <= floor` so a near-zero index cannot divide-by-zero.

A severe single-step reduction is not emitted as a dedicated event; it is
observable through the controller's emitted market-state snapshot.

**Standalone path**: `clean_bad_debt(caller, account_id)`
(`contracts/controller/src/positions/liquidation/mod.rs`) is a **permissionless**
entrypoint for accounts whose bad-debt state needs to be applied outside a
liquidation event. It requires only `caller.require_auth()` (to authenticate the
submitter) plus `require_not_flash_loaning`; it is intentionally **not**
`#[when_not_paused]`, so stranded bad debt can still be crystallized while a
market is paused. Permissionlessness is safe because the call reverts with
`CollateralError::CannotCleanBadDebt` unless `is_socializable_bad_debt` genuinely
holds — a caller can only apply an already-realized loss, never manufacture one.
Keepers are the expected callers but hold no privileged role on this entrypoint.

## Alternatives Considered

- **Auto-pause the market on bad debt.** Rejected: the loss has already
  occurred. Pausing penalizes future suppliers and borrowers and
  obscures the financial reality. The cleanup path emits `CleanBadDebtEvent`
  and updates market state for monitoring; the owner can still pause manually
  if warranted.
- **Insurance fund instead of socialization.** Rejected for launch:
  requires a separate accounting surface and capital provisioning model.
  The current design uses the supply-side claim to express loss.
  Future revenue diverted from `claim_revenue` could fund a
  reserve in a later ADR.
- **No floor on the supply index.** Rejected: a very large bad-debt
  event could drive the index to or near zero, breaking
  `scaled * index / RAY` reconstructions and the revenue-accrual
  divisor.
- **Pro-rata socialization via per-account writes.** Rejected: would
  sweep supply positions at the moment of socialization, which is
  the work the scaled-balance design (ADR 0002) avoids.

## Consequences

Positive:

- Loss attribution is explicit and immediate: suppliers see the index
  step down.
- `CleanBadDebtEvent` and the emitted market-state snapshot give operators a
  high-signal trigger for out-of-band action (pause, communication,
  root-cause).
- Floor preserves the numerical health of all downstream math.
- Per-account work stays at zero; index motion captures the
  socialization.

Negative / accepted costs:

- User-facing risk disclosure must state that suppliers carry socialized losses.
- The `BAD_DEBT_USD_THRESHOLD = $5` heuristic for triggering
  socialization is a tunable; audit and launch review cover threshold
  sensitivity.
- A severe single-step index drop has no dedicated on-chain signal and the
  central pool does not self-pause; operators detect it from the emitted
  market-state snapshot.

## References

- `SCF_BUILD_ARCHITECTURE.md` §6 (Controller Responsibilities), §15 (Security
  Review Focus).
- `contracts/pool/src/interest.rs::apply_bad_debt_to_supply_index`
- `contracts/controller/src/positions/liquidation/apply.rs::check_bad_debt_after_liquidation`
- `contracts/controller/src/positions/liquidation/bad_debt.rs::execute_bad_debt_cleanup`
- `common/src/constants/pool.rs` (`SUPPLY_INDEX_FLOOR_RAW` = `WAD`),
  `contracts/controller/src/constants.rs` (`BAD_DEBT_USD_THRESHOLD` = `5 * WAD`)
- `contracts/controller/src/events/debt.rs::CleanBadDebtEvent`

# Force-socialize bad debt

## Purpose

Clear an insolvent account whose leftover collateral is **above** the
permissionless dust gate (`BAD_DEBT_USD_THRESHOLD` = $5 WAD), so
`clean_bad_debt` reverts with `CannotCleanBadDebt`.

Owner-only. Writes the remaining loss into the affected markets' supply
indexes (ADR-0012). There is no insurance layer; current suppliers take the
hit.

## Preconditions / access

- Caller is the controller **owner** (governance execute of
  `force_socialize_bad_debt`, or a direct owner call in test/admin setups).
- Account exists, has at least one borrow position, and
  `total_debt > total_collateral` (the `Insolvent` gate — no dust cap).
- No flash loan in progress.
- Every remaining position must be priceable (fail-closed; a stale feed
  aborts).

Do **not** raise the dust threshold to paper over a straddle.

## Signals and symptoms

- `is_liquidatable(account_id)` is true, or collateral has already been
  stripped and debt still exceeds collateral.
- `clean_bad_debt` reverts with `CannotCleanBadDebt` (error 114).
- `get_total_collateral_usd > 5e18` and
  `get_total_borrow_usd > get_total_collateral_usd`.
- Typical causes: post-liquidation leftover above $5; `no_seize` making the
  account unliquidatable (ADR-0008 proposed amendment); oracle restored after
  a period of unpriceable dust.

## Immediate checks

1. `get_account_positions` / `get_total_collateral_usd` /
   `get_total_borrow_usd` — confirm insolvency and that collateral is
   **above** $5 (otherwise use permissionless `clean_bad_debt`).
2. `get_market_indexes_detailed` on every remaining asset — no stale/invalid
   legs.
3. Snapshot supply indexes of every remaining **debt** market (those indexes
   will fall).
4. Confirm no in-flight keeper liquidation that could race the same account.

## Standard operating procedure

1. Governance: queue / execute `force_socialize_bad_debt(account_id)`.
2. Expect `CleanBadDebtEvent` and account deletion. No second
   `UpdatePositionBatchEvent` from cleanup.
3. Confirm `account_exists(account_id) == false`.
4. Confirm each affected market's `supply_index` dropped.
5. Record the pre/post index and the USD gap in the incident note. Suppliers
   in those markets have been written down.

## Escalation path

- If the call reverts `CannotCleanBadDebt`: the account is not insolvent
  (debt ≤ collateral). Do not retry; re-read prices and positions.
- If the call reverts on oracle: restore or wait for a valid snapshot; do not
  bypass fail-closed pricing.
- If `no_seize` is why liquidation cannot proceed: socialize only the
  insolvent remainder; do not clear `no_seize` from the guardian path
  (ratchet). Clearing is timelocked `edit_asset_in_spoke`.

## Rollback / recovery

There is no rollback. Socialization is a committed index writedown.
Recapitalize the market (`recapitalize`) if the shortfall must be filled
from treasury; that does not restore the deleted account.

## References

- `contracts/controller/src/positions/liquidation/mod.rs` (`BadDebtGate`)
- `contracts/controller/src/positions/liquidation/curve.rs`
  (`is_socializable_bad_debt`)
- ADR-0012, ADR-0008 proposed amendment
- Pin: `tests/test-harness/tests/controller/bad_debt_index.rs`
  `test_force_socialize_bad_debt_above_dust_threshold`

# Force-socialize bad debt

Scheduling needs the governance `PROPOSER` role. Use this runbook to remove an
insolvent account whose collateral exceeds the $5 dust limit of permissionless
`clean_bad_debt`, including when `no_seize` blocks liquidation.

The account qualifies only when its ceil risk debt is strictly above its half-up
unweighted collateral (`D > C`). A low health factor alone does not qualify. No
dust cap applies.

**Cleanup is irreversible.** All remaining collateral becomes protocol revenue.
All remaining debt is written off against its own markets' supply indexes, even
when collateral and debt share a market. Nothing is netted or paid from
insurance. The cleanup deletes the account and burns its position NFT.

## Before scheduling

Complete these checks before you schedule the operation.

1. Confirm the network, target controller, governance contract, account id,
   NFT owner, positions and spoke.
2. Confirm the authority path. `force_socialize_bad_debt` is owner-only.
   Governance, which is the controller owner, schedules `ForceSocializeBadDebt`
   on the Sensitive delay tier ([governance interface](../endpoints.md#governance)).
3. Read the collateral total with `make <network> getCollateralUsd <account-id>`.
   `get_total_collateral_usd` returns the half-up unweighted collateral
   (`AccountRiskTotals.total_collateral`) the entrypoint uses.
4. Do **not** use `get_total_borrow_usd` for debt. It returns half-up display
   debt. The entrypoint uses ceil-valued debt (`AccountRiskTotals.total_debt`).
   The two values can disagree near the boundary
   ([INV-RISK-02](../invariants.md#inv-risk-02)).
5. Simulate `force_socialize_bad_debt` to check `D > C` exactly.
6. If collateral is at or below $5 and `D > C` holds, stop. Use permissionless
   `clean_bad_debt`, which needs no governance.
7. Check every position's price status. Missing or invalid required prices
   block cleanup. Listing flags and global pause do not waive pricing.
8. Snapshot debt shares, supply shares, revenue, cash, indexes and spoke usage
   for all affected markets.
9. Record the ledger, the time, and the price snapshot.
10. If NFT or account entries are archived, prepare their restoration.
    Ownership lookup and NFT burn both need readable NFT state.
11. Simulate the intended invocation.

## Execute and reconcile

Run these steps in order.

1. Propose `ForceSocializeBadDebt(account_id)` through governance.
2. Immediately before execution, check insolvency again. A repayment or
   liquidation can change eligibility.
3. After the required delay, execute the operation.
4. Confirm the successful transaction and its
   [CleanBadDebtEvent](../events.md#controller-events).
5. Confirm `account_exists(account_id) == false` with
   `make <network> accountExists <account-id>`. Cleanup removes account
   metadata, positions and delegates.
6. Confirm the NFT is absent.
7. Reconcile removed debt shares, reclassified collateral shares, and released
   spoke usage. Account for accrual on touched markets.
8. Record actual supplier-value changes and any remaining backing shortfall.
   A successful cleanup does not establish full backing at the index floor.

Supply indexes need not strictly fall. The floor, zero supply, or accrual can
hide the decrease.

## Failure and recovery

These cases cover failure and recovery.

- `CannotCleanBadDebt`: read positions and prices again. `D > C` must hold.
- `DebtPositionNotFound`: the account has no borrow positions.
- Oracle or storage failure: restore a valid price and entry state, then
  simulate again.
- `FlashLoanOngoing`: the operation cannot run inside a guarded monetary flow.
- `no_seize` does not block this cleanup. Clearing it needs the timelocked
  `relax_spoke_asset_flags`. The guardian and a listing edit cannot clear it.

After a committed cleanup, `recapitalize` can fill an existing backing
shortfall and refund any excess. `recapitalize` does not raise the written-down index or
restore the deleted account. A fully backed market accepts no fill.

See [ADR-0012](../../explanation/decisions.md#adr-0012),
[ADR-0008](../../explanation/decisions.md#adr-0008),
and [INV-LIQ-04](../invariants.md#inv-liq-04--bad-debt-socialization-is-explicit-and-total).

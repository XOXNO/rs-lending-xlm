# Force-socialize bad debt

For protocol operators. You need the governance `PROPOSER` role to schedule the
operation.

This runbook covers governance-authorized removal of an insolvent account.
Use it when collateral exceeds the fixed $5 dust limit of permissionless
`clean_bad_debt`. Cleanup also proceeds when `no_seize` blocks ordinary
liquidation.

The account must hold debt. Its ceil risk debt must be strictly greater than
its half-up unweighted collateral (`D > C`). An unhealthy health factor alone
does not qualify. The forced path has no collateral dust cap.

**Cleanup is irreversible.** All remaining collateral becomes protocol revenue.
All remaining debt is written off against its own markets' supply indexes. This
applies also when collateral and debt share a market. There is no automatic
netting or insurance payment. The account is deleted and its position NFT burns.

## Before scheduling

1. Confirm the network, target controller, governance contract, account id,
   NFT owner, positions and spoke.
2. Confirm the authority path. `force_socialize_bad_debt` is owner-only.
   Governance, the controller owner, schedules `ForceSocializeBadDebt` on the
   Sensitive delay tier. The [governance interface](../endpoints.md#governance)
   describes proposal and execution.
3. Read the collateral total with `make <network> getCollateralUsd <account-id>`.
   `get_total_collateral_usd` returns half-up unweighted collateral
   (`AccountRiskTotals.total_collateral`), the same total the entrypoint uses.
4. Do **not** use `get_total_borrow_usd` for the debt side. It returns half-up
   display debt. The entrypoint uses ceil-valued debt
   (`AccountRiskTotals.total_debt`). The two can disagree near the boundary
   ([INV-RISK-02](../invariants.md#inv-risk-02)).
5. Simulate `force_socialize_bad_debt` to check `D > C` exactly.
6. If collateral is at or below $5 and `D > C` holds, stop here. Use
   permissionless `clean_bad_debt` instead. It needs no governance.
7. Check every position's price status. Missing or invalid required prices
   prevent cleanup. Listing flags and global pause do not waive pricing.
8. Snapshot debt shares, supply shares, revenue, cash, indexes and spoke usage
   for all affected markets.
9. Record the ledger, the time, and the price snapshot.
10. If NFT or account entries are archived, prepare their restoration.
    Ownership lookup and NFT burn both require readable NFT state.
11. Simulate the intended invocation.

## Execute and reconcile

1. Propose `ForceSocializeBadDebt(account_id)` through governance.
2. Immediately before execution, check insolvency again. Another repayment or
   liquidation can change eligibility.
3. After the required delay, execute the operation.
4. Confirm the successful transaction and its
   [CleanBadDebtEvent](../events.md#controller-events). The event records the
   pre-cleanup ceil debt and half-up collateral, in WAD USD. Cleanup emits no
   controller position-update batch.
5. Confirm `account_exists(account_id) == false` with
   `make <network> accountExists <account-id>`. The cleanup removes account
   metadata, positions and delegates.
6. Confirm the NFT is absent.
7. Reconcile removed debt shares, reclassified collateral shares and released
   spoke usage. Account for accrual on touched markets.
8. Record actual supplier-value changes and any remaining backing shortfall.
   A successful cleanup does not establish full backing at the index floor.

Do not require every supply index to strictly fall. The nonzero floor, zero
supplied value, or intervening accrual can hide the decrease.

## Failure and recovery

- `CannotCleanBadDebt`: read positions and prices again. Forced cleanup
  requires ceil risk debt greater than half-up unweighted collateral.
- `DebtPositionNotFound`: the account has no borrow positions.
- Oracle or storage failure: restore a valid price and entry state, then
  simulate again.
- `FlashLoanOngoing`: this operation cannot run inside a guarded monetary flow.
- `no_seize` does not block this cleanup. Clearing `no_seize` requires the
  timelocked `relax_spoke_asset_flags`. The guardian and a listing edit cannot
  clear it.

After a committed cleanup, `recapitalize` can fill an existing backing
shortfall. It refunds any excess. It does not raise the written-down index or
restore the deleted account. A fully backed market accepts no fill.

See [ADR-0012](../../explanation/decisions.md#adr-0012),
[ADR-0008](../../explanation/decisions.md#adr-0008),
and [INV-LIQ-04](../invariants.md#inv-liq-04--bad-debt-socialization-is-explicit-and-total).

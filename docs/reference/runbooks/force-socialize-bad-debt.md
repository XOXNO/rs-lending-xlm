# Force-socialize bad debt

This runbook covers governance-authorized removal of an insolvent account.
Use it when permissionless cleanup is unsuitable, such as when collateral
exceeds the fixed $5 dust limit. Cleanup can also proceed when `no_seize`
blocks ordinary liquidation.

The account must hold debt, with total debt strictly greater than unweighted
collateral value. An unhealthy health factor alone does not qualify. The forced
path has no collateral dust cap.

**Cleanup is irreversible.** All remaining collateral becomes protocol revenue.
All remaining debt is written off against its own markets' supply indexes,
including when collateral and debt share a market. There is no automatic netting
or insurance payment. The account is deleted and its position NFT burns.

## Before scheduling

1. Confirm the network, target controller, account id, NFT owner, positions,
   spoke and deployed governance authority. The controller owner must authorize.
   Governance schedules `ForceSocializeBadDebt` as a sensitive operation; follow
   the [governance interface](../endpoints.md#governance) for proposal and execution.
2. Read `get_total_collateral_usd` and `get_total_borrow_usd` for the account.
   Both return USD WAD. If debt exceeds collateral and collateral is at or below
   $5, permissionless `clean_bad_debt` applies the same cleanup without governance.
3. Check every position's price status. Missing or invalid required prices
   prevent cleanup; listing flags and global pause do not waive pricing.
4. Snapshot debt shares, supply shares, revenue, cash, indexes and spoke usage
   for all affected markets. Preserve the ledger/time and price snapshot.
5. Simulate the intended invocation and prepare any required archived-entry
   restoration. Ownership lookup and NFT burn both require readable NFT state.

## Execute and reconcile

1. Schedule and execute `ForceSocializeBadDebt(account_id)` through governance
   after its required delay. Recheck insolvency immediately before execution;
   another repayment or liquidation can change eligibility.
2. Confirm the successful transaction and [CleanBadDebtEvent](../events.md#controller-events).
   It records pre-cleanup USD totals. Standalone cleanup emits no controller
   position-update batch. Account metadata, positions and delegates are removed,
   and its NFT burns.
3. Confirm `account_exists(account_id) == false` and the NFT is absent.
4. Reconcile removed debt shares, reclassified collateral shares and released
   spoke usage. Account for accrual on touched markets. Do not require every
   supply index to strictly fall: the nonzero floor, zero supplied value or
   intervening accrual can prevent that comparison from showing a decrease.
5. Record actual supplier-value changes and any remaining backing shortfall.
   A successful cleanup does not establish full backing at the index floor.

## Failure and recovery

- `CannotCleanBadDebt`: re-read positions and prices; forced cleanup requires
  debt greater than collateral. An account without borrow positions fails with
  `DebtPositionNotFound`.
- Oracle or storage failure: restore a valid price/entry state and simulate again.
- `FlashLoanOngoing`: this operation cannot run inside a guarded monetary flow.
- Clearing `no_seize` requires timelocked listing editing; the guardian cannot
  clear it. Insolvent cleanup itself bypasses the listing's seizure flag.

After a committed cleanup, `recapitalize` can fill an existing backing shortfall
and refunds excess. It does not raise the written-down index or restore the
deleted account. A fully backed market accepts no fill.

See [ADR-0012](../../explanation/decisions.md#adr-0012),
[ADR-0008](../../explanation/decisions.md#adr-0008),
and [INV-LIQ-04](../invariants.md#inv-liq-04--bad-debt-socialization-is-explicit-and-total).

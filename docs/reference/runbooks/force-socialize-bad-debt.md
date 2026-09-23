# Force-socialize bad debt

This runbook covers governance-authorized removal of an insolvent account.
Use it when collateral exceeds the fixed $5 dust limit of permissionless
`clean_bad_debt`. Cleanup can also proceed when `no_seize` blocks ordinary
liquidation.

The account must hold debt, with ceil risk debt strictly greater than half-up
unweighted collateral (`D > C`). An unhealthy health factor alone does not
qualify. The forced path has no collateral dust cap.

**Cleanup is irreversible.** All remaining collateral becomes protocol revenue.
All remaining debt is written off against its own markets' supply indexes,
including when collateral and debt share a market. There is no automatic netting
or insurance payment. The account is deleted and its position NFT burns.

## Before scheduling

1. Confirm the network, target controller, governance contract, account id,
   NFT owner, positions and spoke. `force_socialize_bad_debt` is owner-only.
   Governance, the controller owner, schedules `ForceSocializeBadDebt` on the
   Sensitive delay tier; follow the
   [governance interface](../endpoints.md#governance) for proposal and execution.
2. Confirm insolvency with the same risk totals the entrypoint uses: ceil-valued
   debt (`AccountRiskTotals.total_debt`) strictly greater than half-up unweighted
   collateral (`AccountRiskTotals.total_collateral`). `get_total_collateral_usd`
   (`make <network> getCollateralUsd <account-id>`) returns that collateral
   total. Do **not** use `get_total_borrow_usd` for the debt side: it returns
   half-up display debt, which can disagree with ceil risk debt near the boundary
   ([INV-RISK-02](../invariants.md#inv-risk-02)). To check `D > C` exactly,
   simulate `force_socialize_bad_debt`. If collateral is at or below $5 and
   `D > C` holds, use permissionless `clean_bad_debt` instead; it needs no
   governance.
3. Check every position's price status. Missing or invalid required prices
   prevent cleanup; listing flags and global pause do not waive pricing.
4. Snapshot debt shares, supply shares, revenue, cash, indexes and spoke usage
   for all affected markets. Preserve the ledger/time and price snapshot.
5. Simulate the intended invocation and prepare any required archived-entry
   restoration. Ownership lookup and NFT burn both require readable NFT state.

## Execute and reconcile

1. Propose `ForceSocializeBadDebt(account_id)` through governance.
2. Recheck insolvency immediately before execution; another repayment or
   liquidation can change eligibility.
3. Execute the operation after its required delay.
4. Confirm the successful transaction and [CleanBadDebtEvent](../events.md#controller-events).
   It records the pre-cleanup ceil debt and half-up collateral, in WAD USD.
   Cleanup emits no controller position-update batch. Account metadata,
   positions and delegates are removed, and its NFT burns.
5. Confirm `account_exists(account_id) == false`
   (`make <network> accountExists <account-id>`) and the NFT is absent.
6. Reconcile removed debt shares, reclassified collateral shares and released
   spoke usage. Account for accrual on touched markets. Do not require every
   supply index to strictly fall: the nonzero floor, zero supplied value or
   intervening accrual can prevent that comparison from showing a decrease.
7. Record actual supplier-value changes and any remaining backing shortfall.
   A successful cleanup does not establish full backing at the index floor.

## Failure and recovery

- `CannotCleanBadDebt`: re-read positions and prices; forced cleanup requires
  ceil risk debt greater than half-up unweighted collateral. An account without
  borrow positions fails with `DebtPositionNotFound`.
- Oracle or storage failure: restore a valid price/entry state and simulate again.
- `FlashLoanOngoing`: this operation cannot run inside a guarded monetary flow.
- Clearing `no_seize` requires the timelocked `relax_spoke_asset_flags`; the
  guardian and a listing edit cannot clear it. Insolvent cleanup itself bypasses
  the listing's seizure flag.

After a committed cleanup, `recapitalize` can fill an existing backing shortfall
and refunds excess. It does not raise the written-down index or restore the
deleted account. A fully backed market accepts no fill.

See [ADR-0012](../../explanation/decisions.md#adr-0012),
[ADR-0008](../../explanation/decisions.md#adr-0008),
and [INV-LIQ-04](../invariants.md#inv-liq-04--bad-debt-socialization-is-explicit-and-total).
